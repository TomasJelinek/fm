//! Immutable file-clipboard copy requests and bounded display events.
use crate::{FileClipboard, FsError, MutationItem, OperationId};
use std::{ffi::OsString, path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub struct CopyRequest {
    pub id: OperationId,
    pub clipboard: FileClipboard,
    pub destination: PathBuf,
}

/// A confirmation retains shared raw clipboard paths; expansion/cloning stays on a worker.
#[derive(Clone, Debug)]
pub struct ClipboardMoveRequest {
    pub id: OperationId,
    pub clipboard: Arc<FileClipboard>,
    pub destination: PathBuf,
}

/// F5 captures the shared selection and projection without expanding them on the UI thread.
#[derive(Clone, Debug)]
pub struct SelectedCopyRequest {
    pub id: OperationId,
    pub files: crate::SelectedFiles,
    pub destination: PathBuf,
}

/// F5 captures virtual identities without interpreting displayed archive paths.
#[derive(Clone, Debug)]
pub struct ArchiveCopyRequest {
    pub id: OperationId,
    pub location: crate::ArchiveLocation,
    pub entries: Arc<[crate::Entry]>,
    /// Visible projection captured with the selection; hidden rows are never targets.
    pub view: Arc<crate::DirectoryIndex>,
    pub selected: Arc<std::collections::HashSet<crate::EntryId>>,
    pub fallback: Option<crate::EntryId>,
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
    ValidatingBeforeMove,
    CapturingSources,
    ValidatingAfterMove,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransferIntent {
    #[default]
    Copy,
    /// Originals are retained in private recovery, never permanently deleted.
    RecoverableMove,
}

#[derive(Clone, Debug)]
pub struct MoveSourceRecovery {
    pub source: PathBuf,
    pub recovery: crate::TransferRecovery,
    /// True only when native identity matches the selected original root.
    pub matched_source: bool,
}

#[derive(Clone, Debug)]
pub struct CopyProgress {
    pub id: OperationId,
    pub intent: TransferIntent,
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
    /// Additional source and target bytes independently read by Move proof passes.
    pub validation_bytes: u64,
    pub source_names_captured: u64,
    pub source_recovery_count: u64,
    /// First16 exact raw source/recovery mappings; count above remains exact.
    pub source_recoveries: Vec<MoveSourceRecovery>,
    /// At most64 source parents to refresh after actual source namespace mutation.
    pub source_directories: Vec<PathBuf>,
    /// Exact number of retained recovery objects; display paths are bounded below.
    pub recovery_count: u64,
    /// First 16 recovery paths. The native provider keeps every object on disk.
    pub recovery_paths: Vec<PathBuf>,
    pub current_path: Option<PathBuf>,
    /// Refresh this captured destination after a terminal event, including partial outcomes.
    pub destination_directory: PathBuf,
    pub message: String,
    /// At most 16 details. `failed` retains the complete error count.
    pub errors: Vec<FsError>,
}

impl CopyProgress {
    pub fn queued(
        id: OperationId,
        destination: PathBuf,
        requested_roots: u64,
        intent: TransferIntent,
    ) -> Self {
        Self {
            id,
            intent,
            state: CopyOperationState::Queued,
            phase: CopyOperationPhase::Planning,
            requested_roots,
            files_completed: 0,
            directories_created: 0,
            skipped: 0,
            failed: 0,
            copied_bytes: 0,
            verified_bytes: 0,
            validation_bytes: 0,
            source_names_captured: 0,
            source_recovery_count: 0,
            source_recoveries: vec![],
            source_directories: vec![],
            recovery_count: 0,
            recovery_paths: vec![],
            current_path: None,
            destination_directory: destination,
            message: String::new(),
            errors: vec![],
        }
    }
}

#[derive(Clone, Debug)]
pub struct CopyConflict {
    pub id: OperationId,
    pub version: u64,
    pub source: MutationItem,
    pub target: MutationItem,
    pub destination: PathBuf,
    pub can_replace: bool,
    pub can_merge: bool,
}

/// Scope is this job only. Recoverable replacement is an explicit choice, never
/// described as atomic conditional overwrite of a consented identity.
#[derive(Clone, Debug)]
pub enum CopyChoice {
    Skip,
    SkipAll,
    Rename(OsString),
    ReplaceRecoverable { all: bool },
    Merge { all: bool },
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
