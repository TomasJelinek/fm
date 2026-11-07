//! Internal checksum contracts. Calculation/export are backend work, not viewer/plugin APIs.
use crate::{EntryKind, FsError, TransferIdentity};
use std::{fmt, path::PathBuf, sync::Arc, time::SystemTime};

pub const MAX_CHECKSUM_JOBS: usize = 32;
pub const CHECKSUM_BATCH_SIZE: usize = 64;
pub const MAX_CHECKSUM_DIAGNOSTICS: usize = 32;
pub const CHECKSUM_BUFFER_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChecksumJobId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChecksumGeneration(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChecksumRowId(pub u64);
/// Monotonic, nonzero engine-session identifier, including rejected saves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChecksumManifestId(pub u64);

#[derive(Clone, Debug)]
pub struct SelectedChecksumRequest {
    pub job: ChecksumJobId,
    pub generation: ChecksumGeneration,
    pub files: crate::SelectedFiles,
    pub algorithms: ChecksumAlgorithms,
    pub recursive: bool,
    pub limits: ChecksumLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}
impl ChecksumAlgorithm {
    pub const ALL: [Self; 4] = [Self::Md5, Self::Sha1, Self::Sha256, Self::Sha512];
    pub const fn bytes(self) -> usize {
        match self {
            Self::Md5 => 16,
            Self::Sha1 => 20,
            Self::Sha256 => 32,
            Self::Sha512 => 64,
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha1 => "SHA-1",
            Self::Sha256 => "SHA-256",
            Self::Sha512 => "SHA-512",
        }
    }
    const fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChecksumAlgorithms(u8);
impl Default for ChecksumAlgorithms {
    fn default() -> Self {
        Self(ChecksumAlgorithm::Sha256.bit())
    }
}
impl ChecksumAlgorithms {
    pub fn new(algorithms: &[ChecksumAlgorithm]) -> Result<Self, &'static str> {
        let mask = algorithms
            .iter()
            .fold(0, |mask, algorithm| mask | algorithm.bit());
        if mask == 0 {
            Err("Choose at least one checksum algorithm.")
        } else {
            Ok(Self(mask))
        }
    }
    pub const fn all() -> Self {
        Self(15)
    }
    pub fn contains(self, algorithm: ChecksumAlgorithm) -> bool {
        self.0 & algorithm.bit() != 0
    }
    pub fn iter(self) -> impl Iterator<Item = ChecksumAlgorithm> {
        ChecksumAlgorithm::ALL
            .into_iter()
            .filter(move |algorithm| self.contains(*algorithm))
    }
}

#[derive(Clone, Debug)]
pub struct ChecksumTarget {
    pub path: PathBuf,
    /// Optional captured listing facts; the backend establishes the native identity.
    pub kind: Option<EntryKind>,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    /// Only a prior backend preparation can supply a meaningful native identity.
    pub identity: Option<TransferIdentity>,
}
impl From<PathBuf> for ChecksumTarget {
    fn from(path: PathBuf) -> Self {
        Self {
            path,
            kind: None,
            size: None,
            modified: None,
            identity: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ChecksumLimits {
    pub max_rows: usize,
    pub max_depth: usize,
    pub max_nodes: u64,
    pub max_retained_bytes: usize,
}
impl Default for ChecksumLimits {
    fn default() -> Self {
        Self {
            max_rows: 16_384,
            max_depth: 64,
            max_nodes: 1_000_000,
            max_retained_bytes: 8 * 1024 * 1024,
        }
    }
}
impl ChecksumLimits {
    pub fn validate(self) -> Result<(), &'static str> {
        let max = Self::default();
        if self.max_rows == 0
            || self.max_rows > max.max_rows
            || self.max_depth == 0
            || self.max_depth > max.max_depth
            || self.max_nodes == 0
            || self.max_nodes > max.max_nodes
            || self.max_retained_bytes == 0
            || self.max_retained_bytes > max.max_retained_bytes
        {
            return Err(
                "Checksum resource limits must be positive and no greater than their defaults.",
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ChecksumRequest {
    pub job: ChecksumJobId,
    pub generation: ChecksumGeneration,
    pub targets: Vec<ChecksumTarget>,
    pub algorithms: ChecksumAlgorithms,
    pub recursive: bool,
    pub limits: ChecksumLimits,
}
impl ChecksumRequest {
    pub fn new(job: ChecksumJobId, generation: ChecksumGeneration, paths: Vec<PathBuf>) -> Self {
        Self {
            job,
            generation,
            targets: paths.into_iter().map(Into::into).collect(),
            algorithms: ChecksumAlgorithms::default(),
            recursive: false,
            limits: ChecksumLimits::default(),
        }
    }
    /// Request validation belongs to the backend admission path, not a UI callback.
    pub fn validate(&self) -> Result<usize, &'static str> {
        use std::path::Component;
        if self.job.0 == 0
            || self.generation.0 == 0
            || self.targets.is_empty()
            || self.targets.len() > 4096
        {
            return Err("Checksums require nonzero job/generation and 1 to 4096 captured targets.");
        }
        let bytes = self
            .targets
            .iter()
            .try_fold(0usize, |total, target| {
                if !target.path.is_absolute()
                    || target.path.as_os_str().as_encoded_bytes().contains(&0)
                    || target
                        .path
                        .components()
                        .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
                {
                    return None;
                }
                total
                    .checked_add(target.path.as_os_str().as_encoded_bytes().len())?
                    .checked_add(std::mem::size_of::<ChecksumTarget>())
            })
            .filter(|bytes| *bytes <= 8 * 1024 * 1024)
            .ok_or(
                "Checksum targets exceed the 8 MiB request budget or contain invalid native paths.",
            )?;
        self.limits.validate()?;
        Ok(bytes.max(1))
    }
}

/// Opaque shared quota ownership retained by every published row/run until it is dropped.
pub struct ChecksumRetention {
    _owner: Arc<dyn Send + Sync>,
}
impl ChecksumRetention {
    pub fn new(owner: impl Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            _owner: Arc::new(owner),
        })
    }
}
impl fmt::Debug for ChecksumRetention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChecksumRetention")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChecksumDigest {
    pub algorithm: ChecksumAlgorithm,
    pub bytes: Vec<u8>,
}
impl ChecksumDigest {
    pub fn hex(&self) -> String {
        self.bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumRowState {
    Calculated,
    SourceChanged,
    ReadError,
    Skipped,
    Cancelled,
}
#[derive(Clone, Debug)]
pub struct ChecksumRow {
    pub id: ChecksumRowId,
    pub path: PathBuf,
    pub identity: Option<TransferIdentity>,
    pub kind: Option<EntryKind>,
    pub state: ChecksumRowState,
    pub bytes_read: u64,
    /// Empty for every state except Calculated.
    pub digests: Vec<ChecksumDigest>,
    pub error: Option<FsError>,
    pub retention: Arc<ChecksumRetention>,
}
impl ChecksumRow {
    pub fn digest(&self, algorithm: ChecksumAlgorithm) -> Option<&ChecksumDigest> {
        (self.state == ChecksumRowState::Calculated)
            .then(|| {
                self.digests
                    .iter()
                    .find(|digest| digest.algorithm == algorithm)
            })
            .flatten()
    }
}
#[derive(Clone, Debug, Default)]
pub struct ChecksumProgress {
    pub discovered: u64,
    pub calculated: u64,
    pub skipped: u64,
    pub errors: u64,
    pub bytes_read: u64,
    pub current: Option<PathBuf>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumCompletion {
    Completed,
    CompletedWithErrors,
    Incomplete,
    Cancelled,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumBudget {
    Rows,
    Depth,
    Nodes,
    RetainedBytes,
}
#[derive(Clone, Debug)]
pub struct ChecksumSummary {
    pub completion: ChecksumCompletion,
    pub progress: ChecksumProgress,
    pub reached_budgets: Vec<ChecksumBudget>,
    pub diagnostics: Vec<FsError>,
}
#[derive(Clone, Debug)]
pub struct ChecksumRun {
    pub job: ChecksumJobId,
    pub generation: ChecksumGeneration,
    pub algorithms: ChecksumAlgorithms,
    pub captured_roots: Arc<[PathBuf]>,
    pub rows: Arc<[Arc<ChecksumRow>]>,
    pub summary: ChecksumSummary,
    pub retention: Arc<ChecksumRetention>,
}
#[derive(Clone, Debug)]
pub enum ChecksumUpdate {
    Queued,
    Started,
    Batch(Arc<[Arc<ChecksumRow>]>),
    Progress(ChecksumProgress),
    Finished(Arc<ChecksumRun>),
}
#[derive(Clone, Debug)]
pub struct ChecksumEvent {
    pub job: ChecksumJobId,
    pub generation: ChecksumGeneration,
    pub update: ChecksumUpdate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChecksumPartialAcknowledgement {
    pub included: u64,
    pub excluded: u64,
    /// Discovery/cancellation may leave additional uncounted work; acknowledgement is explicit.
    pub incomplete_discovery: bool,
}
#[derive(Clone, Debug)]
pub struct ChecksumManifestRequest {
    pub algorithm: ChecksumAlgorithm,
    pub destination: PathBuf,
    pub acknowledge_partial: Option<ChecksumPartialAcknowledgement>,
}
#[derive(Clone, Debug)]
pub struct ChecksumManifestReceipt {
    pub path: PathBuf,
    pub included: u64,
    pub excluded: u64,
    pub verified_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct ChecksumRejected {
    pub job: ChecksumJobId,
    pub generation: ChecksumGeneration,
    pub error: FsError,
}
#[derive(Clone, Debug)]
pub enum ChecksumManifestUpdate {
    /// Admission failure; a duplicate ID must not complete its already-running export.
    Rejected(FsError),
    Queued,
    Saving,
    Finished(Result<ChecksumManifestReceipt, FsError>),
}
#[derive(Clone, Debug)]
pub struct ChecksumManifestEvent {
    pub save_id: ChecksumManifestId,
    pub job: ChecksumJobId,
    pub generation: ChecksumGeneration,
    pub update: ChecksumManifestUpdate,
}
