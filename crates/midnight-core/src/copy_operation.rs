//! Immutable file-clipboard copy requests and bounded display events.
use crate::{FileClipboard, FsError, MutationItem, OperationId};
use std::{ffi::OsString, path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub struct CopyRequest {
    pub verification: crate::TransferVerification,
    pub id: OperationId,
    pub clipboard: FileClipboard,
    pub destination: PathBuf,
}

/// A confirmation retains shared raw clipboard paths; expansion/cloning stays on a worker.
#[derive(Clone, Debug)]
pub struct ClipboardMoveRequest {
    pub verification: crate::TransferVerification,
    pub id: OperationId,
    pub clipboard: Arc<FileClipboard>,
    pub destination: PathBuf,
}

/// F5 captures the shared selection and projection without expanding them on the UI thread.
#[derive(Clone, Debug)]
pub struct SelectedCopyRequest {
    pub verification: crate::TransferVerification,
    pub id: OperationId,
    pub files: crate::SelectedFiles,
    pub destination: PathBuf,
}

/// F5 captures virtual identities without interpreting displayed archive paths.
#[derive(Clone, Debug)]
pub struct ArchiveCopyRequest {
    pub verification: crate::TransferVerification,
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
    /// Each native regular-file source is removed only after its verified copy.
    /// Pre-removal errors restore original names without replacing new winners.
    Move,
}

#[derive(Clone, Debug)]
pub struct MoveSourceResidual {
    pub source: PathBuf,
    pub temporary: crate::TransferRecovery,
    /// True only when this is the selected original left after failed restoration.
    /// False can describe an empty/unknown temporary directory after cleanup failure.
    pub matched_source: bool,
}

#[derive(Clone, Debug)]
pub struct CopyProgress {
    pub id: OperationId,
    pub intent: TransferIntent,
    /// Immutable content-verification choice captured when this job was submitted.
    /// Native rename does not read content regardless of this choice.
    pub verification: crate::TransferVerification,
    pub state: CopyOperationState,
    pub phase: CopyOperationPhase,
    pub requested_roots: u64,
    /// Recursive totals remain unknown; these are observed counters, never estimates.
    pub files_completed: u64,
    pub directories_created: u64,
    /// Actual native entry renames; a renamed directory counts once without
    /// enumerating or estimating its descendants. No content bytes were copied.
    pub native_entries_moved: u64,
    /// One skipped directory means its complete subtree was skipped without enumerating it.
    pub skipped: u64,
    pub failed: u64,
    pub copied_bytes: u64,
    pub verified_bytes: u64,
    /// Additional source and target bytes independently read by Move proof passes.
    pub validation_bytes: u64,
    /// Shared backend samples of writes and independent verification reads.
    pub throughput: Arc<crate::OperationThroughput>,
    /// Internal transient staging count; not a retained-data or completion count.
    pub source_names_staged: u64,
    /// Actual source unlink or accepted normal Windows delete disposition.
    /// This is not a measured physical-space reclamation counter.
    pub source_files_removed: u64,
    pub source_delete_pending: u64,
    /// Completed source namespace changes (unlink or native rename) whose parent
    /// directory durability could not be confirmed by the native platform.
    pub source_removal_durability_unconfirmed: u64,
    /// Exceptional failed restore/cleanup locations only. Normal staging is private.
    pub source_residual_count: u64,
    /// First16 exact raw source/residual mappings; count above remains exact.
    pub source_residuals: Vec<MoveSourceResidual>,
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
            verification: crate::TransferVerification::default(),
            state: CopyOperationState::Queued,
            phase: CopyOperationPhase::Planning,
            requested_roots,
            files_completed: 0,
            directories_created: 0,
            native_entries_moved: 0,
            skipped: 0,
            failed: 0,
            copied_bytes: 0,
            verified_bytes: 0,
            validation_bytes: 0,
            throughput: Arc::default(),
            source_names_staged: 0,
            source_files_removed: 0,
            source_delete_pending: 0,
            source_removal_durability_unconfirmed: 0,
            source_residual_count: 0,
            source_residuals: vec![],
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

/// Failure categories also scope the user's Skip All choice within one operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CopyErrorStage {
    Preparing,
    Reading,
    Writing,
    Syncing,
    Verifying,
    Publishing,
    Enumerating,
    CreatingDirectory,
    RemovingSource,
    Cleanup,
}

/// A worker-owned pause, distinct from overwrite consent. Capabilities describe
/// whether that exact phase can be retried or skipped without replaying a commit.
#[derive(Clone, Debug)]
pub struct CopyFailure {
    pub id: OperationId,
    pub version: u64,
    pub error: crate::FsError,
    pub source: Option<PathBuf>,
    pub destination: PathBuf,
    pub stage: CopyErrorStage,
    pub can_retry: bool,
    pub can_skip: bool,
}

/// Scope is this job only. Recoverable replacement is an explicit choice, never
/// described as atomic conditional overwrite of a consented identity.
#[derive(Clone, Debug)]
pub enum CopyChoice {
    Retry,
    SkipAllErrors,
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
pub enum CompletedCopySource {
    Tree(crate::TreeCopyOccurrence),
    Native(PathBuf),
    Archive(crate::VirtualEntryId),
}

#[derive(Clone, Debug)]
pub enum CopyEvent {
    Progress(Arc<CopyProgress>),
    Conflict(Arc<CopyConflict>),
    Failure(Arc<CopyFailure>),
    /// One requested root completed, including all descendants, with no skips or errors.
    /// Copy requires verified publication; Move also requires source removal.
    RootCompleted {
        id: OperationId,
        source: CompletedCopySource,
    },
}
