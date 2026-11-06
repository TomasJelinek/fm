//! Immutable file-clipboard copy requests and bounded display events.
use crate::{FileClipboard, FsError, MutationItem, OperationId};
use std::{ffi::OsString, path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub struct CopyRequest {
    pub id: OperationId,
    pub clipboard: FileClipboard,
    pub destination: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyOperationState {
    Queued,
    Preparing,
    Running,
    WaitingForDecision,
    CancelRequested,
    Cancelled,
    Completed,
    CompletedWithErrors,
    Failed,
}
impl CopyOperationState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Cancelled | Self::Completed | Self::CompletedWithErrors | Self::Failed
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyOperationPhase {
    Planning,
    Copying,
    Syncing,
    Verifying,
    Publishing,
}

#[derive(Clone, Debug)]
pub struct CopyProgress {
    pub id: OperationId,
    pub state: CopyOperationState,
    pub phase: CopyOperationPhase,
    pub requested_roots: u64,
    /// Recursive totals remain unknown; these are observed counters, never estimates.
    pub files_completed: u64,
    pub directories_created: u64,
    /// One skipped directory means its complete subtree was skipped without enumerating it.
    pub skipped: u64,
    pub failed: u64,
    pub copied_bytes: u64,
    pub verified_bytes: u64,
    pub current_path: Option<PathBuf>,
    /// Refresh this captured destination after a terminal event, including partial outcomes.
    pub destination_directory: PathBuf,
    pub message: String,
    /// At most 16 details. `failed` retains the complete error count.
    pub errors: Vec<FsError>,
}

#[derive(Clone, Debug)]
pub struct CopyConflict {
    pub id: OperationId,
    pub version: u64,
    pub source: MutationItem,
    pub target: MutationItem,
    pub destination: PathBuf,
}

/// Scope is this job only. Replacement is deliberately absent: providers do not
/// implement an atomic conditional overwrite of the identity shown in the dialog.
#[derive(Clone, Debug)]
pub enum CopyChoice {
    Skip,
    SkipAll,
    Rename(OsString),
    Cancel,
}

#[derive(Clone, Debug)]
pub struct CopyDecision {
    pub id: OperationId,
    pub version: u64,
    pub choice: CopyChoice,
}

#[derive(Clone, Debug)]
pub enum CopyEvent {
    Progress(Arc<CopyProgress>),
    Conflict(Arc<CopyConflict>),
}
