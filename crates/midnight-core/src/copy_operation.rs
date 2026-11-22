//! Immutable file-clipboard copy requests and bounded display events.
use crate::{FileClipboard, FsError, MutationItem, OperationId};
use std::{ffi::OsString, path::PathBuf, sync::Arc};

/// Optional dependency waiting, separate from execution capacity and namespace safety.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransferStartPolicy {
    /// No explicit earlier-transfer dependency; normal resource waits still apply.
    #[default]
    Immediate,
    /// Wait for transfers reserved earlier to finish.
    WaitForEarlier,
}

/// Why a transfer is not making progress right now. Shown in its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferWaitReason {
    /// An explicit opt-in dependency on earlier transfers.
    EarlierTransfers,
    /// Execution capacity without a more specific reason.
    Resources,
    /// Every configured disk I/O slot is in use. Running transfers hand theirs over step
    /// by step, so this is brief unless other work keeps the disk busy.
    DiskCapacity,
    /// Another transfer is using the same files or folders: only the overlapping item
    /// waits, or, when this job would read what that one changes, the job waits for it to
    /// finish.
    SameFolders,
    /// This transfer or another one cannot confirm exactly which files and folders it
    /// uses, so that one runs alone and the other waits for it to finish, whichever
    /// folders they use: an unknown namespace footprint, such as names on a Windows
    /// volume that still creates 8.3 short names or on a Linux network filesystem, or
    /// names that a failed check left unconfirmed.
    UnconfirmedFolders,
    /// A file operation that needs every folder to itself (Delete, Rename, a new folder,
    /// a saved file list or checksum file) is running or was started earlier.
    OtherOperation,
}

#[derive(Clone, Debug)]
pub struct CopyRequest {
    pub start_policy: TransferStartPolicy,
    pub verification: crate::TransferVerification,
    pub id: OperationId,
    pub clipboard: FileClipboard,
    pub destination: PathBuf,
}

/// A confirmation retains shared raw clipboard paths; expansion/cloning stays on a worker.
#[derive(Clone, Debug)]
pub struct ClipboardMoveRequest {
    pub start_policy: TransferStartPolicy,
    pub verification: crate::TransferVerification,
    pub id: OperationId,
    pub clipboard: Arc<FileClipboard>,
    pub destination: PathBuf,
}

/// F5 captures the shared selection and projection without expanding them on the UI thread.
#[derive(Clone, Debug)]
pub struct SelectedCopyRequest {
    pub start_policy: TransferStartPolicy,
    pub verification: crate::TransferVerification,
    pub id: OperationId,
    pub files: crate::SelectedFiles,
    pub destination: PathBuf,
}

/// F5 captures virtual identities without interpreting displayed archive paths.
#[derive(Clone, Debug)]
pub struct ArchiveCopyRequest {
    pub start_policy: TransferStartPolicy,
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

/// Token-scoped actions for a temporary native Copy or Move pause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyCancelConfirmationAction {
    Begin,
    Dismiss,
    Confirm,
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

/// Advisory unfinished content work, derived on a worker from discovery and retained
/// copy/proof state. Excludes metadata, sync and namespace latency. Never authority.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransferRemainingWork {
    pub write_bytes: u64,
    /// Initial destination readback plus any remaining Move source/destination proofs.
    pub verification_bytes: u64,
}

/// Final accounting attached to the immutable snapshot that owns the committed
/// totals and bounded diagnostics. Unknown or root-only file totals stay absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyClosureReceipt {
    pub unfinished_roots: u64,
    /// Discovered files without a committed publication, including failed, skipped
    /// and unattempted files.
    pub uncommitted_files: Option<u64>,
    pub uncommitted_bytes: Option<u64>,
    pub omitted_error_details: u64,
}

#[derive(Clone, Debug)]
pub struct CopyProgress {
    pub id: OperationId,
    pub intent: TransferIntent,
    /// Immutable content-verification choice captured when this job was submitted.
    /// Native rename does not read content regardless of this choice.
    pub verification: crate::TransferVerification,
    pub state: CopyOperationState,
    pub wait_reason: Option<TransferWaitReason>,
    /// A request is acknowledged only at a safe backend checkpoint.
    pub pause_requested: bool,
    pub paused: bool,
    /// The active token once the worker is at a safe checkpoint. A dialog can
    /// open only for its own token. Terminal snapshots always clear it.
    pub cancel_confirmation_ready: Option<u64>,
    pub phase: CopyOperationPhase,
    pub requested_roots: u64,
    /// Successfully published selected roots (a directory includes its complete subtree).
    pub completed_roots: u64,
    pub discovery: crate::OperationDiscovery,
    /// None for unsupported/changed discovery, native root scope or invalid accounting.
    pub remaining_work: Option<TransferRemainingWork>,
    /// Logical bytes successfully published once. Unlike copied_bytes this excludes retry I/O.
    pub completed_bytes: u64,
    /// Fully traversed source directories, including merges; excludes skipped/error subtrees.
    pub completed_directories: u64,
    /// Exact current regular-file length and bytes written; absent during traversal/rename.
    /// This is a file-write meter, not an assertion that verification/publication completed.
    pub current_file_size: Option<u64>,
    pub current_file_copied: u64,
    pub current_file_verified: u64,
    /// Zero means no current copied-file attempt. Each new attempt or retry gets a
    /// monotonically increasing nonzero ID within this job; pause preserves it.
    pub verification_pass_id: u64,
    /// How long the current copied-file attempt has been running, measured by the
    /// backend without paused or waiting time; zero without a current attempt. A
    /// window shows a file's own progress only once it takes a while.
    pub current_file_active_millis: u64,
    /// Successful file publications; discovery is an independent metadata snapshot.
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
    /// Current copied-file attempt's destination-readback samples, excluding Move
    /// proof passes. The overall series remains in `throughput`.
    pub verification_throughput: Arc<crate::OperationThroughput>,
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
    /// Exact current target derived from the retained destination and raw source basename.
    pub current_destination: Option<PathBuf>,
    /// Refresh this captured destination after a terminal event, including partial outcomes.
    pub destination_directory: PathBuf,
    pub message: String,
    /// At most 16 details. `failed` retains the complete error count.
    pub errors: Vec<FsError>,
    /// Informational notes for the job details, never errors: for example a link
    /// that Copy recreated as a link because it could not follow it. At most 16;
    /// `note_count` keeps the exact number.
    pub notes: Vec<String>,
    pub note_count: u64,
    /// Links left out, without a question, because the destination creates no links (a
    /// Windows network location, a drive that cannot store links); a note names each
    /// (`TransferDirectory::creates_no_links`). They count neither as copied nor as
    /// skipped: the job completes.
    pub links_left_out: u64,
    /// Present on terminal cancellation, partial completion or recorded issues.
    pub closure: Option<CopyClosureReceipt>,
}

/// The most notes a Copy/Move snapshot carries; `CopyProgress::note_count` is exact.
pub const MAX_COPY_NOTES: usize = 16;

impl CopyProgress {
    /// Record one informational note, keeping the first `MAX_COPY_NOTES`.
    pub fn add_note(&mut self, note: String) {
        self.note_count = self.note_count.saturating_add(1);
        if self.notes.len() < MAX_COPY_NOTES {
            self.notes.push(note);
        }
    }
}

impl CopyProgress {
    /// A placeholder snapshot with the default `verification` policy. A job's
    /// publisher must set `verification` to the job's frozen choice first.
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
            wait_reason: None,
            pause_requested: false,
            paused: false,
            cancel_confirmation_ready: None,
            phase: CopyOperationPhase::Planning,
            requested_roots,
            completed_roots: 0,
            discovery: crate::OperationDiscovery::default(),
            remaining_work: None,
            completed_bytes: 0,
            completed_directories: 0,
            current_file_size: None,
            current_file_copied: 0,
            current_file_verified: 0,
            verification_pass_id: 0,
            current_file_active_millis: 0,
            files_completed: 0,
            directories_created: 0,
            native_entries_moved: 0,
            skipped: 0,
            failed: 0,
            copied_bytes: 0,
            verified_bytes: 0,
            validation_bytes: 0,
            throughput: Arc::default(),
            verification_throughput: Arc::default(),
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
            current_destination: None,
            destination_directory: destination,
            message: String::new(),
            errors: vec![],
            notes: vec![],
            note_count: 0,
            links_left_out: 0,
            closure: None,
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
    /// A Move could not take the source from its folder (a native rename or a
    /// Move-only source check) before anything was copied.
    MovingSource,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_defaults_do_not_claim_a_dependency_wait_or_verification_pass() {
        assert_eq!(
            TransferStartPolicy::default(),
            TransferStartPolicy::Immediate
        );
        for intent in [TransferIntent::Copy, TransferIntent::Move] {
            let progress = CopyProgress::queued(OperationId(1), PathBuf::new(), 2, intent);
            assert_eq!(progress.state, CopyOperationState::Queued);
            assert_eq!(progress.wait_reason, None);
            assert_eq!(progress.verification_pass_id, 0);
            assert!(progress.verification_throughput.samples.is_empty());
            assert_eq!(progress.verification_throughput.transfer_per_second, 0.0);
            assert_eq!(
                progress.verification_throughput.verification_per_second,
                0.0
            );
            assert_eq!(progress.verification_throughput.items_per_second, 0.0);
            assert!(progress.throughput.samples.is_empty());
        }
    }
}
