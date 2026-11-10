//! Exact native clipboard admission and backend-only provider validation.
use crate::{
    providers::{ProviderError, ProviderTarget},
    tree_actor::{CapturedSelection, CapturedTarget, Session},
};
use midnight_core::{
    Cancellation, CapturedClipboardFiles, CapturedDropDirectory, EntryKind, FileClipboard,
    FileClipboardIntent, FsError, TreeClipboardRequest, TreeProjection, TreeRequestId, TreeRowId,
    TreeViewStamp,
};
use std::{
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct Sources {
    capture: CapturedSelection,
    _permit: OwnedSemaphorePermit,
    intent: FileClipboardIntent,
    request_cancel: Cancellation,
    navigation_cancel: Cancellation,
}
impl std::fmt::Debug for Sources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeClipboardSources")
            .field("roots", &self.capture.targets.len())
            .field("intent", &self.intent)
            .finish_non_exhaustive()
    }
}

impl Session {
    pub(crate) fn capture_drop_directory(
        &mut self,
        view: &Arc<TreeProjection>,
        row: TreeRowId,
        request: TreeRequestId,
        capacity: &Arc<Semaphore>,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn CapturedDropDirectory>, ProviderError> {
        if cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if request.0 == 0 || request.0 <= self.last_request.0 {
            return Err(ProviderError::StaleReference);
        }
        let permit = capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProviderError::ResourceLimited)?;
        let captured = self.capture_target(view, row, cancel)?;
        let position = view
            .index
            .row_positions
            .get(&row)
            .ok_or(ProviderError::StaleReference)?;
        if view.rows[*position].entry.kind != EntryKind::Directory
            || !matches!(captured.target.value()?, ProviderTarget::Native(_))
        {
            return Err(ProviderError::Unsupported);
        }
        if cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        self.last_request = request;
        Ok(Arc::new(DropDirectory {
            captured,
            stamp: view.stamp,
            row,
            navigation_cancel: cancel.clone(),
            _permit: permit,
        }))
    }
    pub(crate) fn capture_clipboard(
        &mut self,
        request: &TreeClipboardRequest,
        capacity: &Arc<Semaphore>,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn CapturedClipboardFiles>, ProviderError> {
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
        let capture = match request.scope {
            midnight_core::TreeClipboardScope::SelectionOrFallback => self.capture_selection(
                &request.view,
                request.selection_revision,
                &request.selection,
                request.fallback,
                &request.cancel,
            )?,
            midnight_core::TreeClipboardScope::Dragged(row) => self.capture_dragged(
                &request.view,
                request.selection_revision,
                &request.selection,
                row,
                &request.cancel,
            )?,
        };
        for occurrence in &capture.targets {
            if !matches!(
                occurrence.captured.target.value()?,
                ProviderTarget::Native(_)
            ) {
                return Err(ProviderError::Unsupported);
            }
        }
        if cancel.is_cancelled() || request.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        self.last_request = request.request;
        Ok(Arc::new(Sources {
            capture,
            _permit: permit,
            intent: request.intent,
            request_cancel: request.cancel.clone(),
            navigation_cancel: cancel.clone(),
        }))
    }
}

struct DropDirectory {
    captured: CapturedTarget,
    stamp: TreeViewStamp,
    row: TreeRowId,
    navigation_cancel: Cancellation,
    _permit: OwnedSemaphorePermit,
}
impl std::fmt::Debug for DropDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeDropDirectory")
            .field("stamp", &self.stamp)
            .field("row", &self.row)
            .finish_non_exhaustive()
    }
}
impl CapturedDropDirectory for DropDirectory {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn row(&self) -> TreeRowId {
        self.row
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        if cancel.is_cancelled() || self.navigation_cancel.is_cancelled() {
            return Err(failure(ProviderError::Cancelled));
        }
        let ProviderTarget::Native(path) = self.captured.target.value().map_err(failure)? else {
            return Err(failure(ProviderError::Unsupported));
        };
        FileClipboard {
            paths: vec![path.clone()],
            intent: FileClipboardIntent::Copy,
        }
        .validate()?;
        Ok(path.clone())
    }
}

impl Sources {
    fn check(&self, cancel: &Cancellation) -> Result<(), FsError> {
        if cancel.is_cancelled()
            || self.request_cancel.is_cancelled()
            || self.navigation_cancel.is_cancelled()
        {
            Err(failure(ProviderError::Cancelled))
        } else {
            Ok(())
        }
    }
}
impl CapturedClipboardFiles for Sources {
    fn intent(&self) -> FileClipboardIntent {
        self.intent
    }
    fn tree_copy_occurrence(&self, root: usize) -> Option<midnight_core::TreeCopyOccurrence> {
        self.capture
            .targets
            .get(root)
            .map(|target| midnight_core::TreeCopyOccurrence {
                stamp: self.capture.view.stamp,
                row: target.row,
            })
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<FileClipboard, FsError> {
        self.check(cancel)?;
        let mut paths = Vec::with_capacity(self.capture.targets.len());
        for occurrence in &self.capture.targets {
            self.check(cancel)?;
            let target = occurrence.captured.target.value().map_err(failure)?;
            let ProviderTarget::Native(path) = target else {
                return Err(failure(ProviderError::Unsupported));
            };
            if path.components().next_back() == Some(Component::ParentDir) {
                return Err(failure(ProviderError::InvalidResponse(
                    "Parent navigation is not a clipboard target.",
                )));
            }
            paths.push(path.clone());
        }
        // A later provider validation may revoke an earlier captured context.
        for occurrence in &self.capture.targets {
            self.check(cancel)?;
            occurrence.captured.target.value().map_err(failure)?;
        }
        let files = FileClipboard {
            paths,
            intent: self.intent,
        };
        files.validate()?;
        self.check(cancel)?;
        Ok(files)
    }
}

fn failure(error: ProviderError) -> FsError {
    let kind = match error {
        ProviderError::Cancelled => io::ErrorKind::Interrupted,
        ProviderError::Unsupported => io::ErrorKind::Unsupported,
        _ => io::ErrorKind::InvalidData,
    };
    FsError::from_io("file clipboard", Path::new(""), io::Error::new(kind, error))
}
