//! Typed navigation retains its provider registration until the receiver adopts it.
use super::{RootRegistrationLease, Session, check_cancel};
use crate::providers::{ProviderError, ProviderTarget};
use midnight_core::{
    ArchiveMount, ArchiveNode, Cancellation, EntryKind, FsError, NavigationId,
    TreeNavigationTarget, TreeProjection, TreeRequestId, TreeRowId,
};
use std::sync::Arc;

impl Session {
    #[cfg(test)]
    pub(crate) fn archive_for_taint_test(source: Arc<dyn ArchiveMount>) -> Arc<dyn ArchiveMount> {
        Arc::new(NavigationArchive {
            source: Some(source),
            registration: None,
        })
    }
    pub(crate) fn navigate(
        &mut self,
        view: &Arc<TreeProjection>,
        request: TreeRequestId,
        row: TreeRowId,
        navigation: NavigationId,
        cancel: &Cancellation,
    ) -> Result<TreeNavigationTarget, ProviderError> {
        self.rollback_unpublished();
        check_cancel(cancel)?;
        self.check_row_view(view, row)?;
        if request.0 == 0 || request.0 <= self.last_request.0 || navigation <= view.stamp.navigation
        {
            return Err(ProviderError::StaleReference);
        }
        let branch = self.visible_branch(row)?;
        let position = self.projection.index.row_positions[&row];
        // Context-only directories may be entered; source files and unopened
        // containers require a separate explicit container-attachment contract.
        if !matches!(
            self.projection.rows[position].entry.kind,
            EntryKind::Directory | EntryKind::DirectoryLink
        ) {
            return Err(ProviderError::Unsupported);
        }
        self.tree.navigation_target(branch)?;
        let captured = self.tree.resolve_entry_target(branch, cancel)?;
        let target = match captured.value()? {
            ProviderTarget::Native(path) => TreeNavigationTarget::Native(path.clone()),
            ProviderTarget::Archive(target) => {
                let mut location = target.location.clone();
                let source = location.mount.take().ok_or(ProviderError::Unavailable)?;
                location.mount = Some(Arc::new(NavigationArchive {
                    source: Some(source),
                    registration: Some(self.registration.clone()),
                }));
                TreeNavigationTarget::Archive(location)
            }
        };
        check_cancel(cancel)?;
        self.last_request = request;
        self.stop_recursive();
        Ok(target)
    }
}

struct NavigationArchive {
    source: Option<Arc<dyn ArchiveMount>>,
    registration: Option<Arc<RootRegistrationLease>>,
}
impl NavigationArchive {
    fn source(&self) -> &Arc<dyn ArchiveMount> {
        self.source
            .as_ref()
            .expect("navigation archive remains owned")
    }
    fn release(&mut self) -> Result<(), FsError> {
        use crate::copy_nested_retirement::{self as nested, Retained};
        let mut registration = Retained::new(self.registration.take());
        if let Some(source) = self.source.take() {
            nested::release_archive(source)?;
        }
        if let Some(registration) = registration.take() {
            registration.release_on_worker().map_err(nested::fs_error)?;
        }
        Ok(())
    }
}
impl Drop for NavigationArchive {
    fn drop(&mut self) {
        if let Some(context) = crate::copy_nested_retirement::current()
            && let Err(error) = self.release()
        {
            context.record(error);
        }
    }
}
impl std::fmt::Debug for NavigationArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationArchive")
            .field("id", &self.source().id())
            .finish()
    }
}
impl ArchiveMount for NavigationArchive {
    fn cleanup_unconfirmed(&self) -> bool {
        self.source().cleanup_unconfirmed()
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        let mut reference = crate::copy_nested_retirement::Retained::new(self);
        if reference.get().cleanup_unconfirmed() {
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
    fn id(&self) -> u64 {
        self.source().id()
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.source().list(directory, cancel)
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.source().read(entry, offset, length, cancel)
    }
    // A transparent wrapper: the trait defaults would drain a read the source can
    // cancel cooperatively, and would hide its storage and cache identities. The
    // source is the provider's leased target, which must forward them as well.
    fn read_cooperative(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, FsError> {
        self.source()
            .read_cooperative(entry, offset, length, cancelled)
    }
    fn validate(&self) -> Result<(), FsError> {
        self.source().validate()
    }
    fn native_container_locator(&self) -> Option<&std::path::Path> {
        self.source().native_container_locator()
    }
    fn persistent_member_identity(
        &self,
        entry: u64,
        expected_source_version: &[u8],
    ) -> Result<Option<Vec<u8>>, FsError> {
        self.source()
            .persistent_member_identity(entry, expected_source_version)
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

    /// Records which source methods a navigation target reached.
    #[derive(Debug, Default)]
    struct Recording {
        calls: std::sync::Mutex<Vec<&'static str>>,
        container: std::path::PathBuf,
    }
    impl ArchiveMount for Recording {
        fn id(&self) -> u64 {
            2
        }
        fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
            Ok(Vec::new())
        }
        fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
            self.calls.lock().unwrap().push("read");
            Ok(vec![1])
        }
        fn read_cooperative(
            &self,
            _: u64,
            _: u64,
            _: u32,
            cancelled: &dyn Fn() -> bool,
        ) -> Result<Vec<u8>, FsError> {
            self.calls.lock().unwrap().push("read_cooperative");
            if cancelled() {
                return Err(FsError::from_io(
                    "test read",
                    std::path::Path::new(""),
                    std::io::ErrorKind::Interrupted.into(),
                ));
            }
            Ok(vec![2])
        }
        fn validate(&self) -> Result<(), FsError> {
            Ok(())
        }
        fn native_container_locator(&self) -> Option<&std::path::Path> {
            Some(&self.container)
        }
        fn persistent_member_identity(
            &self,
            entry: u64,
            expected_source_version: &[u8],
        ) -> Result<Option<Vec<u8>>, FsError> {
            Ok(Some(
                [&entry.to_le_bytes()[..], expected_source_version].concat(),
            ))
        }
    }

    #[test]
    fn navigation_archive_forwards_cooperative_reads_and_source_identities() {
        let source = Arc::new(Recording {
            container: std::path::PathBuf::from("container.zip"),
            ..Recording::default()
        });
        let mount = NavigationArchive {
            source: Some(source.clone()),
            registration: None,
        };
        assert_eq!(mount.read_cooperative(7, 0, 1, &|| false).unwrap(), [2]);
        // The source observes the cancellation itself instead of draining a read.
        assert!(mount.read_cooperative(7, 0, 1, &|| true).is_err());
        assert_eq!(
            *source.calls.lock().unwrap(),
            ["read_cooperative", "read_cooperative"]
        );
        assert_eq!(
            mount.native_container_locator(),
            Some(std::path::Path::new("container.zip"))
        );
        assert_eq!(
            mount.persistent_member_identity(7, b"v1").unwrap(),
            Some([&7u64.to_le_bytes()[..], b"v1"].concat())
        );
    }

    #[test]
    fn navigation_archive_drains_shared_and_last_empty_registration() {
        for shared in [true, false] {
            let registration = Arc::new(RootRegistrationLease {
                registry: Some(Arc::new(crate::providers::ProviderRegistry::default())),
                provider: midnight_core::ProviderId::new("retirement-fixture").unwrap(),
                instance: midnight_core::ProviderInstanceId(std::num::NonZeroU64::new(1).unwrap()),
            });
            let outside = shared.then(|| registration.clone());
            let releases = Arc::new(AtomicUsize::new(0));
            let mount = Arc::new(NavigationArchive {
                source: Some(Arc::new(Source(releases.clone()))),
                registration: Some(registration),
            });
            let context = crate::copy_nested_retirement::Context::default();
            let result = context.with(|| mount.release_on_worker());
            assert!(result.is_ok());
            assert!(!context.is_quarantined());
            assert_eq!(releases.load(Ordering::SeqCst), 1);
            drop(outside);
        }
    }
}
