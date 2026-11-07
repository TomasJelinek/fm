//! Bounded read-only recursive Find. All provider calls belong on backend workers.
use crate::{Cancellation, EntryKind, EntryMetadata, FsError, TransferIdentity};
use std::{
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
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
}

#[derive(Clone, Copy, Debug)]
pub struct FindLimits {
    pub max_results: usize,
    pub max_nodes: u64,
    /// Maximum simultaneously retained directory frames, including the root.
    pub max_depth: usize,
    pub max_retained_bytes: usize,
    pub max_content_bytes_per_file: usize,
    pub max_content_bytes: u64,
}
impl Default for FindLimits {
    fn default() -> Self {
        Self {
            max_results: 10_000,
            max_nodes: 1_000_000,
            max_depth: 128,
            max_retained_bytes: 64 * 1024 * 1024,
            max_content_bytes_per_file: 1024 * 1024 * 1024,
            max_content_bytes: 16 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FindRequest {
    pub session: FindSessionId,
    pub generation: FindGeneration,
    pub roots: Vec<PathBuf>,
    pub masks: String,
    pub name_case_sensitive: bool,
    pub recursive: bool,
    pub content: Option<FindContentQuery>,
    pub limits: FindLimits,
}
impl FindRequest {
    pub fn new(session: FindSessionId, generation: FindGeneration, roots: Vec<PathBuf>) -> Self {
        Self {
            session,
            generation,
            roots,
            masks: "*.*".into(),
            name_case_sensitive: false,
            recursive: true,
            content: None,
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
        let max = FindLimits::default();
        if l.max_results == 0
            || l.max_results > max.max_results
            || l.max_nodes == 0
            || l.max_nodes > max.max_nodes
            || l.max_depth == 0
            || l.max_depth > max.max_depth
            || l.max_retained_bytes == 0
            || l.max_retained_bytes > max.max_retained_bytes
            || l.max_content_bytes_per_file == 0
            || l.max_content_bytes_per_file > max.max_content_bytes_per_file
            || l.max_content_bytes == 0
            || l.max_content_bytes > max.max_content_bytes
        {
            return Err(
                "Find resource limits must be positive and no greater than their defaults.",
            );
        }
        Ok(())
    }
}

/// Opaque native revision. Its meaning is private to the supplying OS provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindRevision(pub [u64; 6]);

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
}

#[derive(Clone, Debug)]
pub struct FindResult {
    pub id: FindResultId,
    pub entry: FindEntry,
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

#[derive(Clone, Debug, Default)]
pub struct FindProgress {
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
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindBudget {
    Results,
    Nodes,
    Depth,
    RetainedBytes,
    FileContent,
    TotalContent,
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
    Finished(FindSummary),
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
