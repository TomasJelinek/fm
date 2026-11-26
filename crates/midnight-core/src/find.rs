//! Bounded read-only recursive Find. All provider calls belong on backend workers.
#[path = "find_feed_build.rs"]
mod feed_build;
pub use feed_build::*;
#[path = "find_feed_projection.rs"]
mod feed_projection;
use crate::{Cancellation, EntryKind, EntryMetadata, FsError, TransferIdentity};
pub use feed_projection::*;
use std::{
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub const MAX_FIND_ROOTS: usize = 16;
pub const MAX_FIND_SESSIONS: usize = 4;
pub const FIND_BATCH_SIZE: usize = 64;
pub const MAX_FIND_DIAGNOSTICS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FindSessionId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FindGeneration(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindResultId(pub u64);

#[derive(Clone, Debug)]
pub struct FindContentQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_words: bool,
    /// `text` is a bounded regular expression matched within each line, instead
    /// of literal text. It is compiled on the search worker.
    pub regex: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct FindLimits {
    pub max_results: usize,
    /// Maximum examined native entries and archive members; zero scans without a count limit.
    pub max_nodes: u64,
    /// Maximum simultaneously retained directory frames, including the root.
    pub max_depth: usize,
    pub max_retained_bytes: usize,
    pub max_content_bytes_per_file: usize,
    pub max_content_bytes: u64,
    pub max_duplicate_candidates: usize,
}
impl Default for FindLimits {
    fn default() -> Self {
        Self {
            max_results: 100_000,
            max_nodes: 0,
            max_depth: 128,
            max_retained_bytes: 64 * 1024 * 1024,
            max_content_bytes_per_file: 1024 * 1024 * 1024,
            max_content_bytes: 16 * 1024 * 1024 * 1024,
            max_duplicate_candidates: 100_000,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FindRequest {
    pub session: FindSessionId,
    pub generation: FindGeneration,
    pub roots: Vec<PathBuf>,
    pub masks: String,
    /// Masks by default; Regex and typed metadata Expression use the shared compiler.
    pub name_mode: crate::FilterMode,
    pub sort: FindSortSpec,
    pub name_case_sensitive: bool,
    pub recursive: bool,
    pub content: Option<FindContentQuery>,
    /// Inclusive age interval ending at backend admission, excluding future/unknown times.
    pub modified_within: Option<Duration>,
    /// Distinct native identities with equal bytes; hard-link aliases count once.
    pub duplicates: bool,
    /// Also search the members of archives found on the way (names, and contents
    /// when a content query is set) through the installed archive provider.
    /// Identical files searches native files only.
    pub archives: bool,
    pub limits: FindLimits,
}
impl FindRequest {
    pub fn new(session: FindSessionId, generation: FindGeneration, roots: Vec<PathBuf>) -> Self {
        Self {
            session,
            generation,
            roots,
            masks: "*.*".into(),
            name_mode: crate::FilterMode::Masks,
            sort: FindSortSpec::default(),
            name_case_sensitive: false,
            recursive: true,
            content: None,
            modified_within: None,
            duplicates: false,
            archives: false,
            limits: FindLimits::default(),
        }
    }
    /// Bounds only; mask syntax is compiled on a worker. Native providers validate
    /// their own root syntax without normalizing or reconstructing native paths.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.roots.is_empty() || self.roots.len() > MAX_FIND_ROOTS {
            return Err("Find requires 1 to 16 absolute native directory roots.");
        }
        if self.roots.iter().any(|path| {
            !path.is_absolute()
                || path.as_os_str().as_encoded_bytes().contains(&0)
                || path
                    .components()
                    .any(|part| matches!(part, Component::ParentDir))
        }) {
            return Err("Find roots must be absolute native paths without NUL or parent steps.");
        }
        if self
            .roots
            .iter()
            .try_fold(0usize, |sum, path| {
                sum.checked_add(path.as_os_str().as_encoded_bytes().len())
            })
            .is_none_or(|sum| sum > 1024 * 1024)
        {
            return Err("Find roots exceed the aggregate 1 MiB request budget.");
        }
        if self.masks.len() > 4096 || self.masks.contains('\0') {
            return Err("Find masks exceed 4096 UTF-8 bytes or contain NUL.");
        }
        if self.content.as_ref().is_some_and(|query| {
            query.text.is_empty() || query.text.len() > 4096 || query.text.contains('\0')
        }) {
            return Err("Find content must contain 1 to 4096 UTF-8 bytes without NUL.");
        }
        let l = self.limits;
        if self.modified_within.is_some_and(|duration| {
            duration.is_zero() || duration > Duration::from_secs(366 * 24 * 60 * 60)
        }) {
            return Err("Find modified age must be positive and no greater than 366 days.");
        }
        let max = FindLimits::default();
        if l.max_results == 0
            || l.max_results > max.max_results
            || l.max_depth == 0
            || l.max_depth > max.max_depth
            || l.max_retained_bytes == 0
            || l.max_retained_bytes > max.max_retained_bytes
            || l.max_content_bytes_per_file == 0
            || l.max_content_bytes_per_file > max.max_content_bytes_per_file
            || l.max_content_bytes == 0
            || l.max_content_bytes > max.max_content_bytes
            || l.max_duplicate_candidates == 0
            || l.max_duplicate_candidates > max.max_duplicate_candidates
        {
            return Err(
                "Find result, depth, memory and content limits must be positive and no greater than their defaults.",
            );
        }
        Ok(())
    }
}

/// Native revision of a file. Its values are private to the supplying OS provider, except
/// for where the change time lives (`CHANGE_TIME`), which every provider of a platform
/// lays out alike:
///
/// - Windows: size, LastWriteTime, CreationTime, attributes (without the cloud-state
///   ones), ChangeTime, 0.
/// - Linux: size, mtime seconds, mtime nanoseconds, ctime seconds, ctime nanoseconds,
///   mode (file type and permission bits).
///
/// Directories carry no revision (all zero).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindRevision(pub [u64; 6]);

impl FindRevision {
    /// The slots that hold the change time (NTFS ChangeTime, Linux ctime).
    #[cfg(windows)]
    pub const CHANGE_TIME: &'static [usize] = &[4];
    /// The slots that hold the change time (NTFS ChangeTime, Linux ctime).
    #[cfg(not(windows))]
    pub const CHANGE_TIME: &'static [usize] = &[3, 4];

    /// Every fact but the change time is equal. The change time also moves on
    /// metadata-only updates that leave the content alone: endpoint protection attaches
    /// extended attributes (a new NTFS ChangeTime) to a file within a second of its first
    /// read, and on Linux xattr, chmod, chown and link-count changes move the ctime. Find
    /// treats such a difference as unchanged, as viewers do (`RevisionPolicy::
    /// AcceptChangeTimeOnly` of the plugin host); a change of mode or attributes still
    /// counts, since those are compared slots.
    pub fn same_except_change_time(&self, other: &Self) -> bool {
        self.0
            .iter()
            .zip(other.0)
            .enumerate()
            .all(|(slot, (mine, theirs))| *mine == theirs || Self::CHANGE_TIME.contains(&slot))
    }
}

#[derive(Clone, Debug)]
pub struct FindEntry {
    pub path: PathBuf,
    pub kind: EntryKind,
    pub metadata: EntryMetadata,
    pub identity: TransferIdentity,
    pub revision: FindRevision,
}
impl FindEntry {
    pub fn matches_capture(&self, captured: &Self) -> bool {
        self.path == captured.path
            && self.kind == captured.kind
            && self.identity == captured.identity
            && self.revision == captured.revision
    }

    /// `matches_capture`, except that a difference in the change time alone is accepted
    /// (`FindRevision::same_except_change_time`): Find's own revalidation of what it
    /// captured, and of its results.
    pub fn matches_capture_except_change_time(&self, captured: &Self) -> bool {
        self.path == captured.path
            && self.kind == captured.kind
            && self.identity == captured.identity
            && self.revision.same_except_change_time(&captured.revision)
    }
}

#[derive(Clone, Debug)]
pub struct FindResult {
    pub id: FindResultId,
    /// For an archive member, `path` is the archive's native path joined with the
    /// member's names: presentation and sorting only, never a native target.
    pub entry: FindEntry,
    /// Stable within this search; every published group contains at least two identities.
    pub duplicate_group: Option<u64>,
    /// Full group size, or zero for ordinary results.
    pub duplicate_group_size: usize,
    /// Set for a member found inside an archive (Search inside archives).
    pub archive: Option<Arc<FindArchiveMember>>,
}

/// Where a member found inside an archive lives. The provider address names the
/// member's folder; the member is chosen there by its raw name.
#[derive(Clone, Debug)]
pub struct FindArchiveMember {
    pub folder: crate::ArchiveAddress,
    pub name: Vec<u8>,
    /// The native archive file as it was searched; Go to file revalidates it.
    pub container: FindEntry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindSkipReason {
    Link,
    Special,
    MountBoundary,
}
pub enum FindChild {
    Entry(Box<dyn FindSource>),
    Skipped {
        path: PathBuf,
        reason: FindSkipReason,
    },
    /// The failed child was consumed; the next call advances to another child.
    Failed(FsError),
}

/// Each source pins one ordinary native file or directory, without following its
/// final component. Child traversal never follows links or crosses a mount/volume.
pub trait FindSource: Send {
    fn entry(&self) -> &FindEntry;
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
    fn next_child(&mut self, cancel: &Cancellation) -> Result<Option<FindChild>, FsError>;
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError>;
}
pub trait FindService: Send + Sync + 'static {
    /// Open an ordinary file/directory for search or later result validation.
    /// Starting filesystem roots are supported; link roots fail explicitly.
    fn open_find_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn FindSource>, FsError>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FindPhase {
    #[default]
    Scanning,
    HashingDuplicates,
    ComparingDuplicates,
}

#[derive(Clone, Debug, Default)]
pub struct FindProgress {
    pub phase: FindPhase,
    pub examined: u64,
    pub directories: u64,
    pub files: u64,
    pub content_bytes: u64,
    pub results: u64,
    pub skipped_links: u64,
    pub skipped_special: u64,
    pub skipped_mounts: u64,
    pub errors: u64,
    pub limited_files: u64,
    pub duplicate_candidates: u64,
    pub duplicate_groups: u64,
    /// Actual hash/compare bytes, also charged to content_bytes and its shared limit.
    pub duplicate_bytes: u64,
    /// Search inside archives: archives opened, and members examined in them. Shown as
    /// progress only; no limit applies to either (members count as visited entries).
    pub archives: u64,
    pub archive_members: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindBudget {
    Results,
    Nodes,
    Depth,
    RetainedBytes,
    FileContent,
    TotalContent,
    DuplicateCandidates,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindCompletion {
    Completed,
    Incomplete,
    Cancelled,
    Failed,
}
#[derive(Clone, Debug)]
pub struct FindSummary {
    pub completion: FindCompletion,
    pub progress: FindProgress,
    pub reached_budgets: Vec<FindBudget>,
    /// At most MAX_FIND_DIAGNOSTICS samples. Counts above include unsampled errors.
    pub diagnostics: Vec<FsError>,
}
#[derive(Clone, Debug)]
pub enum FindUpdate {
    Started,
    Batch(Arc<[Arc<FindResult>]>),
    Progress(FindProgress),
    /// Raw scan location (directory during traversal, file during duplicate
    /// hashing/comparison), sampled no more than four times per second even
    /// while native I/O blocks. Never an operation target.
    CurrentDirectory(Arc<PathBuf>),
    Sorted(Arc<FindOrder>),
    SortFailed {
        revision: u64,
        error: FsError,
    },
    Finished(FindSummary),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FindSortColumn {
    #[default]
    Name,
    Extension,
    Path,
    Size,
    Modified,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindSortSpec {
    pub column: FindSortColumn,
    pub descending: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct FindSortRequest {
    pub session: FindSessionId,
    pub generation: FindGeneration,
    pub revision: u64,
    pub sort: FindSortSpec,
}
/// Bounded backend-built view; discovery-order result authority is never reordered.
/// `positions[id-1]` is the row of that stable ID in `ids`.
#[derive(Clone, Debug)]
pub struct FindOrder {
    pub revision: u64,
    pub sort: FindSortSpec,
    pub ids: Arc<[FindResultId]>,
    pub positions: Arc<[usize]>,
}
#[derive(Clone, Copy, Debug)]
pub struct FindValidationRequest {
    pub session: FindSessionId,
    pub generation: FindGeneration,
    pub result: FindResultId,
    pub token: u64,
}

#[derive(Clone, Debug)]
pub struct FindEvent {
    pub session: FindSessionId,
    pub generation: FindGeneration,
    pub update: FindUpdate,
}
#[derive(Clone, Debug)]
pub struct FindValidationResult {
    pub request: FindValidationRequest,
    pub result: Result<Box<FindEntry>, FsError>,
}
#[derive(Clone, Debug)]
pub struct FindRejected {
    pub session: FindSessionId,
    pub generation: FindGeneration,
    pub error: FsError,
}

pub fn find_error(path: &Path, kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io("find files", path, io::Error::new(kind, message))
}

/// A separately captured file capability, never a navigation validation receipt.
/// Cached accessors are UI-safe. Validation and final provider destruction belong
/// to backend workers. Restart/Close revoke the originating generation; Stop does not.
pub trait CapturedFindFile: std::fmt::Debug + Send + Sync {
    fn request(&self) -> FindValidationRequest;
    fn entry(&self) -> &FindEntry;
    fn validate(&self, cancel: &Cancellation) -> Result<(), FsError>;
}

#[derive(Clone, Debug)]
pub struct FindFileCaptureResult {
    pub request: FindValidationRequest,
    pub result: Result<Arc<dyn CapturedFindFile>, FsError>,
}

#[derive(Clone, Debug)]
pub enum FindFileOperation {
    Open,
    Edit(crate::EditorPreferences),
}
