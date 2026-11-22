//! Internal checksum contracts. Calculation/export are backend work, not viewer/plugin APIs.
//! A run has no size limit: it holds any number of files at any depth, so its rows are
//! compact (see [`ChecksumRow`]) and shared in chunks between the engine and the window.
use crate::{EntryKind, FsError, TransferIdentity};
use std::{
    borrow::Cow,
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

pub const MAX_CHECKSUM_JOBS: usize = 32;
/// Rows a worker sends together (sooner when they finish slowly); a batching size, not a
/// limit.
pub const CHECKSUM_BATCH_SIZE: usize = 64;
/// Diagnostics a run's summary keeps for Details; every failed row keeps its own error.
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
    Crc32,
}
impl ChecksumAlgorithm {
    pub const ALL: [Self; 5] = [
        Self::Md5,
        Self::Sha1,
        Self::Sha256,
        Self::Sha512,
        Self::Crc32,
    ];
    pub const fn bytes(self) -> usize {
        match self {
            Self::Md5 => 16,
            Self::Sha1 => 20,
            Self::Sha256 => 32,
            Self::Sha512 => 64,
            Self::Crc32 => 4,
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha1 => "SHA-1",
            Self::Sha256 => "SHA-256",
            Self::Sha512 => "SHA-512",
            Self::Crc32 => "CRC32",
        }
    }
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
            Self::Crc32 => "sfv",
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
        Self(31)
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

#[derive(Clone, Debug)]
pub struct ChecksumRequest {
    pub job: ChecksumJobId,
    pub generation: ChecksumGeneration,
    pub targets: Vec<ChecksumTarget>,
    pub algorithms: ChecksumAlgorithms,
    pub recursive: bool,
}
impl ChecksumRequest {
    pub fn new(job: ChecksumJobId, generation: ChecksumGeneration, paths: Vec<PathBuf>) -> Self {
        Self {
            job,
            generation,
            targets: paths.into_iter().map(Into::into).collect(),
            algorithms: ChecksumAlgorithms::default(),
            recursive: false,
        }
    }
    /// Request validation belongs to the backend admission path, not a UI callback. It
    /// refuses malformed requests only: a request may name any number of targets.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.job.0 == 0 || self.generation.0 == 0 || self.targets.is_empty() {
            return Err(
                "Checksums require a nonzero job and generation and at least one captured target.",
            );
        }
        if !self
            .targets
            .iter()
            .all(|target| native_absolute(&target.path))
        {
            return Err("Checksum targets must be absolute native paths without traversal or NUL.");
        }
        Ok(())
    }
}

/// An absolute native path without `.`/`..` steps or NUL.
fn native_absolute(path: &Path) -> bool {
    use std::path::Component;
    path.is_absolute()
        && !path.as_os_str().as_encoded_bytes().contains(&0)
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
}

/// A captured native manifest. Expected digests are read and validated by the backend.
#[derive(Clone, Debug)]
pub struct ChecksumVerifyRequest {
    pub job: ChecksumJobId,
    pub generation: ChecksumGeneration,
    pub manifest: PathBuf,
}
impl ChecksumVerifyRequest {
    /// Validate without filesystem access; whole-request admission is backend-owned.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.job.0 == 0 || self.generation.0 == 0 {
            return Err("Checksum verification requires a nonzero job and generation.");
        }
        if !native_absolute(&self.manifest) {
            return Err("Checksum manifest must be an absolute native path without traversal.");
        }
        Ok(())
    }
}

/// Opaque ownership of a job's slot, held by its run and by every chunk of its rows (wherever
/// they are kept) until the last of them is dropped.
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

/// One algorithm's digest, borrowed from the row that holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChecksumDigest<'a> {
    pub algorithm: ChecksumAlgorithm,
    pub bytes: &'a [u8],
}
impl ChecksumDigest<'_> {
    /// Lowercase hexadecimal text, built in one allocation.
    pub fn hex(&self) -> String {
        let mut text = String::with_capacity(self.bytes.len().saturating_mul(2));
        self.push_hex(&mut text);
        text
    }
    /// Append the lowercase hexadecimal text, without an allocation of its own.
    pub fn push_hex(&self, text: &mut String) {
        push_hex(text, self.bytes);
    }
}
fn push_hex(text: &mut String, bytes: &[u8]) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    text.reserve(bytes.len().saturating_mul(2));
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
}

/// A row's digests: each calculated algorithm's fixed-length value, in the order of
/// [`ChecksumAlgorithm::ALL`], together in one allocation of exactly their length.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ChecksumDigests {
    /// The algorithms present, as `ChecksumAlgorithms` bits; zero for none.
    algorithms: u8,
    bytes: Box<[u8]>,
}
impl ChecksumDigests {
    /// These digests, in any order. None when a value does not have its algorithm's
    /// length or an algorithm repeats.
    pub fn new(values: &[(ChecksumAlgorithm, &[u8])]) -> Option<Self> {
        let mut algorithms = 0u8;
        let mut length = 0usize;
        for (algorithm, value) in values {
            if value.len() != algorithm.bytes() || algorithms & algorithm.bit() != 0 {
                return None;
            }
            algorithms |= algorithm.bit();
            length += value.len();
        }
        let mut bytes = Vec::with_capacity(length);
        for algorithm in ChecksumAlgorithm::ALL {
            if let Some((_, value)) = values.iter().find(|(known, _)| *known == algorithm) {
                bytes.extend_from_slice(value);
            }
        }
        Some(Self {
            algorithms,
            bytes: bytes.into_boxed_slice(),
        })
    }
    pub fn is_empty(&self) -> bool {
        self.algorithms == 0
    }
    pub fn get(&self, algorithm: ChecksumAlgorithm) -> Option<ChecksumDigest<'_>> {
        self.iter().find(|digest| digest.algorithm == algorithm)
    }
    /// Every digest, in [`ChecksumAlgorithm::ALL`] order.
    pub fn iter(&self) -> impl Iterator<Item = ChecksumDigest<'_>> + '_ {
        let mut offset = 0;
        ChecksumAlgorithm::ALL
            .into_iter()
            .filter(|algorithm| self.algorithms & algorithm.bit() != 0)
            .map(move |algorithm| {
                let start = offset;
                offset += algorithm.bytes();
                ChecksumDigest {
                    algorithm,
                    bytes: &self.bytes[start..offset],
                }
            })
    }
}
impl fmt::Debug for ChecksumDigests {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(
                self.iter()
                    .map(|digest| format!("{} {}", digest.algorithm.label(), digest.hex())),
            )
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumRowState {
    Calculated,
    Verified,
    Mismatch,
    Missing,
    SourceChanged,
    ReadError,
    Skipped,
    Cancelled,
}
/// One result row. A run has no size limit, so a row is compact: its name below its
/// chunk's base folder, its digests in one fixed-length allocation, an error only when
/// it has one, and no display text (the window formats what it shows on demand).
#[derive(Clone, Debug)]
pub struct ChecksumRow {
    pub id: ChecksumRowId,
    /// The path below its chunk's [`ChecksumChunk::base`]; a path that is not below it is
    /// kept whole (joining it to the base gives it back unchanged).
    pub name: Box<Path>,
    pub identity: Option<TransferIdentity>,
    pub kind: Option<EntryKind>,
    pub state: ChecksumRowState,
    pub bytes_read: u64,
    /// Complete actual digests for Calculated, Verified and Mismatch; empty on read failures.
    pub digests: ChecksumDigests,
    pub error: Option<Box<FsError>>,
}
impl ChecksumRow {
    pub fn digest(&self, algorithm: ChecksumAlgorithm) -> Option<ChecksumDigest<'_>> {
        matches!(
            self.state,
            ChecksumRowState::Calculated | ChecksumRowState::Verified | ChecksumRowState::Mismatch
        )
        .then(|| self.digests.get(algorithm))
        .flatten()
    }
}

/// Rows a worker found together: one allocation for all of them, their base folder and
/// the job's ownership, shared (never copied) by the engine's retained run and the window.
#[derive(Debug)]
pub struct ChecksumChunk {
    /// The folder every row's name is below.
    pub base: Arc<Path>,
    pub rows: Box<[ChecksumRow]>,
    /// The job's ownership, released once every chunk of its rows is gone.
    pub retention: Arc<ChecksumRetention>,
}

/// A row with the base folder its name is below.
#[derive(Clone, Copy, Debug)]
pub struct ChecksumRowRef<'a> {
    pub base: &'a Path,
    pub row: &'a ChecksumRow,
}
impl ChecksumRowRef<'_> {
    /// The row's whole native path.
    pub fn path(&self) -> PathBuf {
        self.base.join(&self.row.name)
    }
}
impl std::ops::Deref for ChecksumRowRef<'_> {
    type Target = ChecksumRow;
    fn deref(&self) -> &ChecksumRow {
        self.row
    }
}

/// A run's rows in the order found, as shared chunks. Cloning shares the chunks.
#[derive(Clone, Debug, Default)]
pub struct ChecksumRows {
    chunks: Vec<Arc<ChecksumChunk>>,
    /// The index of each chunk's first row.
    starts: Vec<usize>,
    len: usize,
}
impl std::ops::Index<usize> for ChecksumRows {
    type Output = ChecksumRow;
    /// The row at `index`, as a slice gives it: there must be one.
    fn index(&self, index: usize) -> &ChecksumRow {
        match self.get(index) {
            Some(row) => row.row,
            None => panic!("row {index} of {} checksum rows", self.len),
        }
    }
}
impl From<Arc<ChecksumChunk>> for ChecksumRows {
    fn from(chunk: Arc<ChecksumChunk>) -> Self {
        let mut rows = Self::default();
        rows.push(chunk);
        rows
    }
}
impl ChecksumRows {
    pub fn push(&mut self, chunk: Arc<ChecksumChunk>) {
        if chunk.rows.is_empty() {
            return;
        }
        self.starts.push(self.len);
        self.len += chunk.rows.len();
        self.chunks.push(chunk);
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn chunks(&self) -> &[Arc<ChecksumChunk>] {
        &self.chunks
    }
    /// The row at `index`, found without a scan.
    pub fn get(&self, index: usize) -> Option<ChecksumRowRef<'_>> {
        if index >= self.len {
            return None;
        }
        let position = self.starts.partition_point(|start| *start <= index) - 1;
        let chunk = &self.chunks[position];
        Some(ChecksumRowRef {
            base: &chunk.base,
            row: chunk.rows.get(index - self.starts[position])?,
        })
    }
    /// The row whose ID is `id`: a worker numbers its rows from one in the order found.
    pub fn by_id(&self, id: u64) -> Option<ChecksumRowRef<'_>> {
        let index = usize::try_from(id.checked_sub(1)?).ok()?;
        self.get(index).filter(|row| row.id.0 == id)
    }
    pub fn iter(&self) -> impl Iterator<Item = ChecksumRowRef<'_>> + '_ {
        self.chunks.iter().flat_map(|chunk| {
            chunk.rows.iter().map(|row| ChecksumRowRef {
                base: &chunk.base,
                row,
            })
        })
    }
}

/// The name of `name` (below `base`) relative to a checksum file's `folder`, without an
/// allocation when the folder is the base or below it. None when it is not below the folder.
pub fn checksum_file_name<'a>(base: &Path, name: &'a Path, folder: &Path) -> Option<Cow<'a, Path>> {
    if name.is_absolute() {
        return name.strip_prefix(folder).ok().map(Cow::Borrowed);
    }
    if let Ok(below) = folder.strip_prefix(base) {
        return name.strip_prefix(below).ok().map(Cow::Borrowed);
    }
    let above = base.strip_prefix(folder).ok()?;
    Some(Cow::Owned(above.join(name)))
}
#[derive(Clone, Debug, Default)]
pub struct ChecksumProgress {
    pub discovered: u64,
    pub calculated: u64,
    pub verified: u64,
    pub mismatched: u64,
    pub missing: u64,
    pub skipped: u64,
    pub errors: u64,
    pub bytes_read: u64,
    pub current: Option<PathBuf>,
    /// A calculation lists its files before reading them: how many it listed and
    /// their total length (zero while unknown, and for verification).
    pub planned: u64,
    pub planned_bytes: u64,
    /// The calculation is still listing its files; nothing has been read yet.
    pub listing: bool,
    /// Bytes read of `current` so far, and its length.
    pub current_read: u64,
    pub current_size: Option<u64>,
}

/// Why a file has no checksum-file line: it is not below the checksum file's folder.
pub const CHECKSUM_NAME_NOT_BELOW: &str =
    "Manifest sources must be below its containing directory on the same native volume.";

/// One line of a checksum file for `path` below `directory`, exactly as a saved file
/// has it: GNU binary style `<hex> *<name>`, with a leading backslash when the name
/// escapes `\`, CR or LF, or SFV `<name>  <hex>` for CRC32. The name is relative to
/// `directory`, with `/` separators. The error says why the path cannot be written.
pub fn checksum_file_line(
    algorithm: ChecksumAlgorithm,
    hex: &str,
    path: &Path,
    directory: &Path,
) -> Result<String, (std::io::ErrorKind, &'static str)> {
    let relative = path
        .strip_prefix(directory)
        .map_err(|_| (std::io::ErrorKind::InvalidInput, CHECKSUM_NAME_NOT_BELOW))?;
    let mut line = String::new();
    write_checksum_file_line(&mut line, algorithm, |text| text.push_str(hex), relative)?;
    Ok(line)
}

/// Append the checksum-file line of a file `name`d relative to the checksum file's
/// folder (see [`checksum_file_name`]): the text [`checksum_file_line`] returns, written
/// straight into `text`. On an error nothing is appended.
pub fn push_checksum_file_line(
    text: &mut String,
    digest: ChecksumDigest<'_>,
    name: &Path,
) -> Result<(), (std::io::ErrorKind, &'static str)> {
    write_checksum_file_line(text, digest.algorithm, |text| digest.push_hex(text), name)
}

fn write_checksum_file_line(
    text: &mut String,
    algorithm: ChecksumAlgorithm,
    hex: impl Fn(&mut String),
    name: &Path,
) -> Result<(), (std::io::ErrorKind, &'static str)> {
    let start = text.len();
    let written = write_line(text, algorithm, hex, name);
    if written.is_err() {
        text.truncate(start);
    }
    written
}

fn write_line(
    text: &mut String,
    algorithm: ChecksumAlgorithm,
    hex: impl Fn(&mut String),
    name: &Path,
) -> Result<(), (std::io::ErrorKind, &'static str)> {
    use std::{io::ErrorKind, path::Component};
    let traversal = (
        ErrorKind::InvalidInput,
        "Manifest names cannot contain traversal, roots or volume prefixes.",
    );
    let is_sfv = algorithm == ChecksumAlgorithm::Crc32;
    // GNU marks a line whose name escapes `\`, CR or LF with a leading backslash.
    let mut escaped = false;
    let mut parts = 0usize;
    for component in name.components() {
        let Component::Normal(component) = component else {
            return Err(traversal);
        };
        escaped |= component
            .as_encoded_bytes()
            .iter()
            .any(|byte| matches!(byte, b'\\' | b'\r' | b'\n'));
        parts += 1;
    }
    if parts == 0 {
        return Err((ErrorKind::InvalidInput, "Empty relative manifest filename."));
    }
    let escaped = escaped && !is_sfv;
    if !is_sfv {
        if escaped {
            text.push('\\');
        }
        hex(text);
        text.push_str(" *");
    }
    let name_start = text.len();
    for (index, component) in name.components().enumerate() {
        let Component::Normal(component) = component else {
            return Err(traversal);
        };
        let part = component.to_str().ok_or((
            ErrorKind::InvalidData,
            "Native filename cannot round-trip through the UTF-8 GNU manifest format.",
        ))?;
        if index > 0 {
            text.push('/');
        }
        if escaped {
            for character in part.chars() {
                match character {
                    '\\' => text.push_str("\\\\"),
                    '\n' => text.push_str("\\n"),
                    '\r' => text.push_str("\\r"),
                    other => text.push(other),
                }
            }
        } else {
            text.push_str(part);
        }
    }
    if is_sfv {
        let written = &text[name_start..];
        if written.contains(['\\', '\r', '\n', '\0'])
            || written.trim_start().starts_with([';', '#'])
            || written.starts_with(char::is_whitespace)
            || written.ends_with([' ', '\t'])
        {
            return Err((
                ErrorKind::InvalidInput,
                "This filename cannot be represented unambiguously in SFV format.",
            ));
        }
        text.push_str("  ");
        hex(text);
    }
    text.push('\n');
    Ok(())
}

/// A file a calculation will read, listed before reading starts, in reading order: its
/// name below the listing's base folder, and its length when listed (reading checks it
/// again).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChecksumPlannedFile {
    pub name: Box<Path>,
    pub size: Option<u64>,
}
/// Files listed together, below one base folder (the base of the run's rows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChecksumPlanned {
    pub base: Arc<Path>,
    pub files: Box<[ChecksumPlannedFile]>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumCompletion {
    Completed,
    CompletedWithErrors,
    /// Some files could not be listed (an unreadable folder, a changed source), so the
    /// run may have more files than it found.
    Incomplete,
    Cancelled,
    Failed,
}
#[derive(Clone, Debug)]
pub struct ChecksumSummary {
    pub completion: ChecksumCompletion,
    pub progress: ChecksumProgress,
    pub diagnostics: Vec<FsError>,
}
#[derive(Clone, Debug)]
pub struct ChecksumRun {
    pub job: ChecksumJobId,
    pub generation: ChecksumGeneration,
    pub algorithms: ChecksumAlgorithms,
    pub captured_roots: Arc<[PathBuf]>,
    pub rows: ChecksumRows,
    pub summary: ChecksumSummary,
    pub retention: Arc<ChecksumRetention>,
}
#[derive(Clone, Debug)]
pub enum ChecksumUpdate {
    Queued,
    Started,
    /// Emitted only after the complete manifest has been read and validated.
    Verifying(ChecksumAlgorithm),
    /// Files a calculation will read, in batches before reading starts. Rows follow
    /// for each, in the same order, plus rows for entries that cannot be read.
    Planned(Arc<ChecksumPlanned>),
    /// The next rows, shared with the run that finishes with them.
    Batch(Arc<ChecksumChunk>),
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
    /// Replace an existing regular file after the user confirmed it. Without this an
    /// existing destination fails with `AlreadyExists` so the caller can ask first.
    pub overwrite: bool,
}
#[derive(Clone, Debug)]
pub struct ChecksumManifestReceipt {
    pub path: PathBuf,
    pub included: u64,
    pub excluded: u64,
    pub verified_bytes: u64,
    /// A replaced file the platform kept for recovery; user data, never removed here.
    pub recovery: Vec<PathBuf>,
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
