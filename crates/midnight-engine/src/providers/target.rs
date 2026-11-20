//! Backend target leases, including independently cloned archive mount handles.
use super::{MountedLocation, ProviderError, check_cancel};
use crate::providers::ProviderTarget;
use midnight_core::{ArchiveMount, ArchiveNode, Cancellation, FsError, NodeRef};
use std::{fmt, io, path::PathBuf, sync::Arc};

struct TargetContext(Vec<(MountedLocation, NodeRef)>);

impl TargetContext {
    fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        crate::copy_nested_retirement::provider_release(
            self,
            |context| match Arc::into_inner(context) {
                Some(owner) => owner.release_locations(),
                None => Ok(()),
            },
            Self::give_up,
        )
    }
    /// Releases the locations in order. A failed release leaves this retirement
    /// unconfirmed, so every location after it is given up instead of released.
    fn release_locations(self) -> Result<(), ProviderError> {
        use crate::copy_nested_retirement::Retained;
        let mut locations = Retained::new(self.0.into_iter());
        let mut result = Ok(());
        while let Some((location, _)) = locations.get_mut().next() {
            if result.is_ok() {
                result = location.release_on_worker();
            } else {
                location.give_up();
            }
        }
        drop(locations.take());
        result
    }
    /// Decrement-only: a shared reference only gives up its count, and the last one
    /// gives up each location's lease, whose last owner keeps its registration and
    /// mount permit retained.
    fn give_up(self: Arc<Self>) {
        use crate::copy_nested_retirement::Retained;
        let Some(owner) = Arc::into_inner(self) else {
            return;
        };
        let mut locations = Retained::new(owner.0.into_iter());
        while let Some((location, _)) = locations.get_mut().next() {
            location.give_up();
        }
        drop(locations.take());
    }
    /// Resolution only: every context node must still be issued by its lease.
    fn validate(&self) -> Result<(), ProviderError> {
        for (location, node) in &self.0 {
            location.validate_reference(node)?;
        }
        Ok(())
    }

    /// A resolved target's registrations: each lease is unrevoked and each node
    /// belongs to it. Not current issuance, which the mount's lister may reclaim.
    fn validate_registration(&self) -> Result<(), ProviderError> {
        for (location, node) in &self.0 {
            location.lease.check_registration(node)?;
        }
        Ok(())
    }

    fn request(&self, cancel: &Cancellation) -> Cancellation {
        self.0
            .iter()
            .fold(cancel.clone(), |request, (location, _)| {
                location.lease.lifetime.linked(&request)
            })
    }
}

/// Retains the provider context while the backend resolves an exact entry.
/// This is not a mutation proof; the operation must establish its own authority.
pub struct MountedTarget {
    // Provider resources are destroyed before their registration context.
    target: ProviderTarget,
    context: Arc<TargetContext>,
}

impl MountedTarget {
    #[cfg(test)]
    pub(crate) fn archive_for_taint_test(source: Arc<dyn ArchiveMount>) -> Arc<dyn ArchiveMount> {
        Arc::new(LeasedArchive {
            source: Some(source),
            context: Some(Arc::new(TargetContext(Vec::new()))),
            path: PathBuf::new(),
        })
    }
    /// Releases the archive source before the target context. When that release fails
    /// (a tainted session), the context is given up instead of released, so its last
    /// owner still passes the tree's archive reference on.
    pub(crate) fn release_on_worker(self) -> Result<(), ProviderError> {
        use crate::copy_nested_retirement::{self as nested, Retained};
        nested::provider_release(
            self,
            |owner| {
                let mut context = Retained::new(owner.context);
                if let ProviderTarget::Archive(mut archive) = owner.target
                    && let Some(source) = archive.location.mount.take()
                    && let Err(error) = nested::release_archive(source)
                {
                    context.take().give_up();
                    return Err(ProviderError::Failed(error.to_string()));
                }
                context.take().release_on_worker()
            },
            Self::give_up,
        )
    }
    /// Where the release cannot run: gives up the archive source, then the target
    /// context. Decrement-only; the context's last mount lease keeps its registration
    /// and mount permit retained.
    pub(crate) fn give_up(self) {
        use crate::copy_nested_retirement::{self as nested, Retained};
        let mut context = Retained::new(self.context);
        if let ProviderTarget::Archive(mut archive) = self.target
            && let Some(source) = archive.location.mount.take()
        {
            nested::give_up_archive(source);
        }
        context.take().give_up();
    }
    /// Validates registration lifetime and mount identity of the target and every
    /// enclosing context, not current issuance, source revision or a
    /// panel-selection snapshot: a resolved target keeps its value after its
    /// lister releases the node references. Cloned archive handles retain the same
    /// context and reject its revocation.
    pub fn value(&self) -> Result<&ProviderTarget, ProviderError> {
        self.context.validate_registration()?;
        Ok(&self.target)
    }
}

impl MountedLocation {
    pub fn resolve_target(
        &self,
        node: &NodeRef,
        cancel: &Cancellation,
    ) -> Result<MountedTarget, ProviderError> {
        self.resolve_target_with_context(node, cancel, Vec::new())
    }

    pub(in crate::providers) fn resolve_target_with_context(
        &self,
        node: &NodeRef,
        cancel: &Cancellation,
        mut ancestors: Vec<(MountedLocation, NodeRef)>,
    ) -> Result<MountedTarget, ProviderError> {
        // Two contexts per ancestor: the source entry and its optional container.
        if ancestors.len() > 128 {
            return Err(ProviderError::ResourceLimited);
        }
        ancestors.push((self.clone(), node.clone()));
        let context = Arc::new(TargetContext(ancestors));
        context.validate()?;
        let request = context.request(cancel);
        check_cancel(&request)?;
        let mut target = self.lease.mount.resolve_target(&node.token, &request)?;
        check_cancel(&request)?;
        context.validate()?;
        if let ProviderTarget::Archive(archive) = &mut target {
            let source = archive
                .location
                .mount
                .take()
                .ok_or(ProviderError::InvalidResponse(
                    "resolved archive target has no live mount",
                ))?;
            archive.location.mount = Some(Arc::new(LeasedArchive {
                source: Some(source),
                context: Some(context.clone()),
                path: archive.location.address.container.clone(),
            }));
        }
        Ok(MountedTarget { target, context })
    }
}

struct LeasedArchive {
    source: Option<Arc<dyn ArchiveMount>>,
    context: Option<Arc<TargetContext>>,
    path: PathBuf,
}

impl fmt::Debug for LeasedArchive {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LeasedArchive")
            .field("id", &self.source().id())
            .finish_non_exhaustive()
    }
}

impl LeasedArchive {
    fn source(&self) -> &Arc<dyn ArchiveMount> {
        self.source.as_ref().expect("archive source remains owned")
    }
    fn context(&self) -> &TargetContext {
        self.context.as_ref().expect("archive target remains owned")
    }
    /// A failed source release hands the target context down by give-up, never by an
    /// ordinary release that would follow an unconfirmed resource teardown.
    fn release(&mut self) -> Result<(), FsError> {
        use crate::copy_nested_retirement::{self as nested, Retained};
        let mut context = Retained::new(self.context.take());
        if let Some(source) = self.source.take()
            && let Err(error) = nested::release_archive(source)
        {
            if let Some(context) = context.take() {
                context.give_up();
            }
            return Err(error);
        }
        if let Some(context) = context.take() {
            context.release_on_worker().map_err(nested::fs_error)?;
        }
        Ok(())
    }
    /// Last owner of a tainted or quarantined release: `release` gives up the source,
    /// then the target context is given up (decrement-only; its last mount lease keeps
    /// its registration and permit retained), so the tree's capture of the session is
    /// not pinned.
    fn give_up(&mut self, release: impl FnOnce(Arc<dyn ArchiveMount>)) {
        use crate::copy_nested_retirement::Retained;
        let mut context = Retained::new(self.context.take());
        if let Some(source) = self.source.take() {
            release(source);
        }
        if let Some(context) = context.take() {
            context.give_up();
        }
    }
    fn check(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.context()
            .validate_registration()
            .and_then(|()| check_cancel(cancel))
            .map_err(|error| {
                FsError::from_io("archive target", &self.path, io::Error::other(error))
            })
    }
}
impl Drop for LeasedArchive {
    fn drop(&mut self) {
        if let Some(context) = crate::copy_nested_retirement::current()
            && let Err(error) = self.release()
        {
            context.record(error);
        }
    }
}

impl ArchiveMount for LeasedArchive {
    fn cleanup_unconfirmed(&self) -> bool {
        self.source().cleanup_unconfirmed()
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        let mut reference = crate::copy_nested_retirement::Retained::new(self);
        if reference.get().cleanup_unconfirmed() {
            // Never destroyed: a shared reference only gives up its count, and the last
            // one hands its source to the source's own failing release, so the
            // session's last owner still reaches the host's.
            if let Some(mut owner) = Arc::into_inner(reference.take()) {
                owner.give_up(|source| {
                    let _ = source.release_on_worker();
                });
            }
            return Err(crate::copy_nested_retirement::unconfirmed());
        }
        let Some(mut owner) = Arc::into_inner(reference.take()) else {
            return Ok(());
        };
        let context = crate::copy_nested_retirement::current().unwrap_or_default();
        let result = context.with(|| owner.release());
        if let Err(error) = &result {
            context.record(error.clone());
        }
        result
    }
    fn release_quarantined(self: Arc<Self>) {
        if let Some(mut owner) = Arc::into_inner(self) {
            owner.give_up(|source| source.release_quarantined());
        }
    }
    fn id(&self) -> u64 {
        self.source().id()
    }

    // The source is the shared archive session. A revoked or cancelled listing ends
    // between pages; member reads stay cooperative so that revoking this target's
    // registrations never terminates the worker serving other owners (archive_shared.rs).
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        let request = self.context().request(cancel);
        self.check(&request)?;
        let rows = self.source().list(directory, &request);
        self.check(&request)?;
        rows
    }

    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.read_cooperative(entry, offset, length, &|| cancel.is_cancelled())
    }

    fn read_cooperative(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, FsError> {
        let request = self.context().request(&Cancellation::default());
        self.check(&request)?;
        let bytes = self.source().read_cooperative(entry, offset, length, &|| {
            cancelled() || request.is_cancelled()
        });
        self.check(&request)?;
        bytes
    }

    fn validate(&self) -> Result<(), FsError> {
        let cancel = Cancellation::default();
        self.check(&cancel)?;
        let result = self.source().validate();
        self.check(&cancel)?;
        result
    }

    // Archive previews classify the container's storage by this locator and key
    // their persistent caches by the member identity; the trait defaults would hide
    // both. A revoked target vouches for neither.
    fn native_container_locator(&self) -> Option<&std::path::Path> {
        self.context().validate_registration().ok()?;
        self.source().native_container_locator()
    }

    fn persistent_member_identity(
        &self,
        entry: u64,
        expected_source_version: &[u8],
    ) -> Result<Option<Vec<u8>>, FsError> {
        let cancel = Cancellation::default();
        self.check(&cancel)?;
        let identity = self
            .source()
            .persistent_member_identity(entry, expected_source_version);
        self.check(&cancel)?;
        identity
    }

    // An archive inside this archive reports its enclosing folder and opens from a
    // member staged here; a revoked target vouches for neither.
    fn parent(&self) -> Option<midnight_core::ArchiveParent> {
        self.context().validate_registration().ok()?;
        self.source().parent()
    }

    fn stage_member(
        &self,
        entry: u64,
        cancelled: &dyn Fn() -> bool,
        progress: &dyn Fn(u64),
    ) -> Result<Arc<dyn midnight_core::ArchiveContent>, FsError> {
        let request = self.context().request(&Cancellation::default());
        self.check(&request)?;
        let content =
            self.source()
                .stage_member(entry, &|| cancelled() || request.is_cancelled(), progress);
        self.check(&request)?;
        content
    }
}

#[cfg(test)]
mod retirement_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Debug)]
    struct Source(Arc<AtomicUsize>);
    impl ArchiveMount for Source {
        fn id(&self) -> u64 {
            1
        }
        fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
            Ok(Vec::new())
        }
        fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
            Ok(Vec::new())
        }
        fn validate(&self) -> Result<(), FsError> {
            Ok(())
        }
        fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
            if let Some(source) = Arc::into_inner(self) {
                source.0.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }
    #[test]
    fn leased_archive_forwards_consuming_release_before_its_empty_context() {
        let releases = Arc::new(AtomicUsize::new(0));
        let mount = Arc::new(LeasedArchive {
            source: Some(Arc::new(Source(releases.clone()))),
            context: Some(Arc::new(TargetContext(Vec::new()))),
            path: PathBuf::new(),
        });
        mount.release_on_worker().unwrap();
        assert_eq!(releases.load(Ordering::SeqCst), 1);
    }
}
