//! Typed navigation retains its provider registration until the receiver adopts it.
use super::{RootRegistrationLease, Session, check_cancel};
use crate::providers::{ProviderError, ProviderTarget};
use midnight_core::{
    ArchiveMount, ArchiveNode, Cancellation, EntryKind, FsError, NavigationId,
    TreeNavigationTarget, TreeProjection, TreeRequestId, TreeRowId,
};
use std::sync::Arc;

impl Session {
    pub(crate) fn navigate(
        &mut self,
        view: &Arc<TreeProjection>,
        request: TreeRequestId,
        row: TreeRowId,
        navigation: NavigationId,
        cancel: &Cancellation,
    ) -> Result<TreeNavigationTarget, ProviderError> {
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
                    source,
                    _registration: self.registration.clone(),
                }));
                TreeNavigationTarget::Archive(location)
            }
        };
        check_cancel(cancel)?;
        self.last_request = request;
        Ok(target)
    }
}

struct NavigationArchive {
    source: Arc<dyn ArchiveMount>,
    _registration: Arc<RootRegistrationLease>,
}
impl std::fmt::Debug for NavigationArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationArchive")
            .field("id", &self.source.id())
            .finish()
    }
}
impl ArchiveMount for NavigationArchive {
    fn id(&self) -> u64 {
        self.source.id()
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.source.list(directory, cancel)
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.source.read(entry, offset, length, cancel)
    }
    fn validate(&self) -> Result<(), FsError> {
        self.source.validate()
    }
}
