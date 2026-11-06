//! Immutable mutation requests and bounded display summaries; no OS handles cross UI.
use crate::{
    DirectoryIndex, EntryId, FsError, MutationItem, MutationKind, SelectionSnapshot,
    VisibleDirectory,
};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperationId(pub u64);

#[derive(Clone, Debug)]
pub struct PrepareOperation {
    pub id: OperationId,
    pub kind: MutationKind,
    pub source: VisibleDirectory,
    pub view: Arc<DirectoryIndex>,
    pub selection: Arc<SelectionSnapshot>,
    pub focused: Option<EntryId>,
}

#[derive(Clone, Debug)]
pub struct OperationPreview {
    pub id: OperationId,
    pub kind: MutationKind,
    pub files: u64,
    pub folders: u64,
    /// At most 32 entries; the complete prepared target list stays on the worker.
    pub sample: Vec<MutationItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationState {
    Preparing,
    AwaitingConfirmation,
    Queued,
    Running,
    CancelRequested,
    Cancelled,
    Completed,
    CompletedWithErrors,
    Failed,
}

impl OperationState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Cancelled | Self::Completed | Self::CompletedWithErrors | Self::Failed
        )
    }
}

#[derive(Clone, Debug)]
pub struct OperationProgress {
    pub id: OperationId,
    pub kind: MutationKind,
    pub state: OperationState,
    /// Top-level accepted targets; not the unknown recursive number of entries.
    pub total: u64,
    pub completed: u64,
    pub failed: u64,
    /// Removed nodes for deletion; the final entry for rename/create (not intermediate dirs).
    pub processed_entries: u64,
    pub current_path: Option<PathBuf>,
    pub message: String,
    /// At most 16 errors; failed remains the full top-level failure count.
    pub errors: Vec<FsError>,
    pub source_directory: PathBuf,
    pub renamed_to: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum OperationEvent {
    Preview(Arc<OperationPreview>),
    Progress(Arc<OperationProgress>),
}
