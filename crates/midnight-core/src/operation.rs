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
    /// Sum of prepared top-level regular-file lengths, excluding folder contents.
    /// None means a non-directory target has unknown size/kind or the sum overflowed.
    pub file_bytes: Option<u64>,
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
    /// A request is acknowledged only at a safe backend checkpoint.
    pub pause_requested: bool,
    pub paused: bool,
    /// Top-level accepted targets; not the unknown recursive number of entries.
    pub total: u64,
    /// Top-level targets whose change completed. A Rename or Delete target that also
    /// reports an issue (a leftover staging folder or a required recovery) counts here
    /// and in `failed`, so `completed + failed` may exceed `total`. A metadata edit
    /// counts a failed target, including one whose rename left a staging folder,
    /// only in `failed`.
    pub completed: u64,
    pub discovery: crate::OperationDiscovery,
    /// Top-level targets with an issue: targets that did not complete, plus completed
    /// targets that reported one (see `completed`). A Delete counts its errors instead:
    /// each failure that was skipped (one entry left, with the folders that contain
    /// it) or that ended its target, and each completed target's issue. A target kept
    /// only because it still contains skipped items is said so once in `message`, not
    /// counted again.
    pub failed: u64,
    /// Removed nodes for deletion; the final entry for rename/create (not intermediate dirs).
    pub processed_entries: u64,
    /// Shared backend activity samples; deletion measures processed entries/s.
    pub throughput: Arc<crate::OperationThroughput>,
    pub current_path: Option<PathBuf>,
    pub message: String,
    /// At most 16 errors; `failed` remains the full top-level issue count. An error may
    /// describe a completed target's leftover staging folder or required recovery.
    pub errors: Vec<FsError>,
    pub source_directory: PathBuf,
    pub renamed_to: Option<PathBuf>,
}

/// A Delete paused on an entry it could not remove, until the user decides with
/// `Command::DecideDelete` (Retry, Skip, SkipAllErrors or Cancel, for this version).
#[derive(Clone, Debug)]
pub struct DeleteFailure {
    pub id: OperationId,
    pub version: u64,
    pub mode: crate::DeleteMode,
    /// Only the original pinned targets which the provider explicitly could not recycle.
    pub permanent_fallback: Option<OperationPreview>,
    /// Why, naming the entry, or the folder whose entries cannot be removed.
    pub error: FsError,
    pub can_retry: bool,
    pub can_skip: bool,
}

#[derive(Clone, Debug)]
pub enum OperationEvent {
    Preview(Arc<OperationPreview>),
    Progress(Arc<OperationProgress>),
    MetadataPreview(Arc<crate::MetadataPreview>),
    MetadataResult(Arc<crate::MetadataResult>),
    MetadataConflict(Arc<crate::MetadataConflict>),
    RenameConflict(Arc<crate::CopyConflict>),
    DeleteFailure(Arc<DeleteFailure>),
}
