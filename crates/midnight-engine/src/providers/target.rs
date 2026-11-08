//! Backend target leases, including independently cloned archive mount handles.
use super::{MountedLocation, ProviderError, check_cancel};
use crate::providers::ProviderTarget;
use midnight_core::{ArchiveMount, ArchiveNode, Cancellation, FsError, NodeRef};
use std::{fmt, io, path::PathBuf, sync::Arc};

struct TargetContext(Vec<(MountedLocation, NodeRef)>);

impl TargetContext {
    fn validate(&self) -> Result<(), ProviderError> {
        for (location, node) in &self.0 {
            location.validate_reference(node)?;
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
    /// Validates registrations, not source revision or a panel-selection snapshot.
    /// Cloned archive handles retain the same context and reject its revocation.
    pub fn value(&self) -> Result<&ProviderTarget, ProviderError> {
        self.context.validate()?;
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
                source,
                context: context.clone(),
                path: archive.location.address.container.clone(),
            }));
        }
        Ok(MountedTarget { target, context })
    }
}

struct LeasedArchive {
    source: Arc<dyn ArchiveMount>,
    context: Arc<TargetContext>,
    path: PathBuf,
}

impl fmt::Debug for LeasedArchive {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LeasedArchive")
            .field("id", &self.source.id())
            .finish_non_exhaustive()
    }
}

impl LeasedArchive {
    fn check(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.context
            .validate()
            .and_then(|()| check_cancel(cancel))
            .map_err(|error| {
                FsError::from_io("archive target", &self.path, io::Error::other(error))
            })
    }
}

impl ArchiveMount for LeasedArchive {
    fn id(&self) -> u64 {
        self.source.id()
    }

    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        let request = self.context.request(cancel);
        self.check(&request)?;
        let rows = self.source.list(directory, &request);
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
        let request = self.context.request(cancel);
        self.check(&request)?;
        let bytes = self.source.read(entry, offset, length, &request);
        self.check(&request)?;
        bytes
    }

    fn validate(&self) -> Result<(), FsError> {
        let cancel = Cancellation::default();
        self.check(&cancel)?;
        let result = self.source.validate();
        self.check(&cancel)?;
        result
    }
}
