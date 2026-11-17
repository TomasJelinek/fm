//! F4 admission retains the provider-resolved focused occurrence, not selection.
use crate::{
    providers::{ProviderError, ProviderTarget},
    tree_actor::{CapturedTarget, Session, capture_failure, validate_native_leaf},
};
use midnight_core::{
    Cancellation, CapturedEditorFile, EntryKind, FsError, TreeEditorRequest, TreeRowId,
    TreeViewStamp,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct Source {
    captured: CapturedTarget,
    stamp: TreeViewStamp,
    row: TreeRowId,
    request_cancel: Cancellation,
    navigation_cancel: Cancellation,
    _permit: OwnedSemaphorePermit,
}
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeEditorFile")
            .field("stamp", &self.stamp)
            .field("row", &self.row)
            .finish()
    }
}

impl Session {
    pub(crate) fn capture_editor(
        &mut self,
        request: &TreeEditorRequest,
        capacity: &Arc<Semaphore>,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn CapturedEditorFile>, ProviderError> {
        if cancel.is_cancelled() || request.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if request.request.0 == 0 || request.request.0 <= self.last_request.0 {
            return Err(ProviderError::StaleReference);
        }
        let permit = capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProviderError::ResourceLimited)?;
        let linked = cancel.linked(&request.cancel);
        let captured = self.capture_target(&request.view, request.row, &linked)?;
        let position = request
            .view
            .index
            .row_positions
            .get(&request.row)
            .ok_or(ProviderError::StaleReference)?;
        if request.view.rows[*position].entry.kind != EntryKind::File {
            return Err(ProviderError::Unsupported);
        }
        let ProviderTarget::Native(path) = captured.target.value()? else {
            return Err(ProviderError::Unsupported);
        };
        validate_path(path)?;
        if linked.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        self.last_request = request.request;
        Ok(Arc::new(Source {
            captured,
            stamp: request.view.stamp,
            row: request.row,
            request_cancel: request.cancel.clone(),
            navigation_cancel: cancel.clone(),
            _permit: permit,
        }))
    }
}

impl CapturedEditorFile for Source {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn row(&self) -> TreeRowId {
        self.row
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        let check = || {
            if cancel.is_cancelled()
                || self.request_cancel.is_cancelled()
                || self.navigation_cancel.is_cancelled()
            {
                Err(failure(ProviderError::Cancelled))
            } else {
                Ok(())
            }
        };
        check()?;
        let ProviderTarget::Native(path) = self.captured.target.value().map_err(failure)? else {
            return Err(failure(ProviderError::Unsupported));
        };
        validate_path(path).map_err(failure)?;
        check()?;
        Ok(path.clone())
    }
}

/// A file occurrence is always a leaf; the same rule as Rename and metadata.
fn validate_path(path: &Path) -> Result<(), ProviderError> {
    validate_native_leaf(
        path,
        "Editor target must be an absolute native file, not parent navigation.",
    )
}

fn failure(error: ProviderError) -> FsError {
    capture_failure("external editor capture", error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_capture_rejects_parent_navigation_but_preserves_authorized_internal_parent_components()
     {
        let root = std::env::current_dir().unwrap();
        for path in [
            PathBuf::from("relative.txt"),
            root.join(".."),
            root.join("nul\0.txt"),
        ] {
            assert!(validate_path(&path).is_err(), "{path:?}");
        }
        assert!(validate_path(&root.join("文档 & $HOME.txt")).is_ok());
        assert!(validate_path(&root.join("folder").join("..").join("file.txt")).is_ok());
    }
}
