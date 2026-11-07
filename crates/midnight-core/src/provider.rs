use crate::{Entry, EntryMetadata};
use std::{
    ffi::OsString,
    fmt, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug, Default)]
pub struct Cancellation {
    local: Arc<AtomicBool>,
    parents: Arc<[Arc<AtomicBool>]>,
}

impl Cancellation {
    pub fn cancel(&self) {
        self.local.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.local.load(Ordering::Acquire)
            || self.parents.iter().any(|flag| flag.load(Ordering::Acquire))
    }

    /// Cancellation of any ancestor is visible immediately, including inside an
    /// OS callback. Cancelling the child never cancels its parent or siblings.
    pub fn child(&self) -> Self {
        let mut parents = Vec::with_capacity(self.parents.len() + 1);
        parents.push(self.local.clone());
        parents.extend(self.parents.iter().cloned());
        Self {
            local: Arc::default(),
            parents: parents.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FsError {
    pub operation: &'static str,
    pub path: PathBuf,
    pub message: String,
    pub kind: io::ErrorKind,
}

impl FsError {
    pub fn from_io(operation: &'static str, path: &Path, error: io::Error) -> Self {
        Self {
            operation,
            path: path.to_owned(),
            message: error.to_string(),
            kind: error.kind(),
        }
    }
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} ({})",
            self.operation,
            self.message,
            self.path.display()
        )
    }
}
impl std::error::Error for FsError {}

#[derive(Clone, Debug)]
pub struct Volume {
    pub id: String,
    pub label: String,
    pub root: PathBuf,
}

#[derive(Clone, Copy, Debug)]
pub struct SpaceInfo {
    pub available: u64,
    pub total: u64,
}

/// Independently queried display details; one failure never hides the other field.
#[derive(Clone, Debug)]
pub struct VolumeDetails {
    pub label: Result<Option<OsString>, FsError>,
    pub space: Result<SpaceInfo, FsError>,
}

/// Conservative scheduling hint, never a guarantee of physical device topology.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StorageKind {
    LocalSsd,
    Rotational,
    Network,
    #[default]
    Unknown,
}

/// Blocking directory enumeration and metadata access; call on a worker thread.
/// Paths and names retain their native identity without Unicode normalization.
pub trait DirectoryService: Send + Sync + 'static {
    /// Streams names and types, optionally with cached metadata, without following links.
    /// Cancellation or a false callback return stops enumeration cooperatively.
    ///
    /// # Errors
    /// Returns a contextual error when the directory or an entry cannot be read.
    /// Entries that disappear during enumeration may be skipped.
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError>;

    /// True when enumeration supplied this provider's complete display metadata.
    /// This is a listing snapshot, never authority for later mutation or transfer.
    fn listing_metadata_complete(&self, _entry: &Entry) -> bool {
        false
    }

    /// Reads metadata without following the final link or opening special-file contents.
    ///
    /// # Errors
    /// Returns a contextual error when the entry is missing or inaccessible.
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError>;
}

/// Blocking discovery of navigable roots, independent of capacity queries.
pub trait VolumeService: Send + Sync + 'static {
    /// Lists roots without waiting for storage-capacity lookups on individual volumes.
    ///
    /// # Errors
    /// Returns a contextual error if the platform cannot enumerate volumes.
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError>;

    /// Returns the native volume name without changing the root's raw identity.
    /// Call on a worker separately from cheap root discovery.
    ///
    /// # Errors
    /// Returns a contextual error for unavailable or inaccessible volume metadata.
    fn volume_label(&self, _path: &Path) -> Result<Option<OsString>, FsError> {
        Ok(None)
    }
}

/// Blocking storage-capacity queries; call on a worker thread separately from listing.
pub trait StorageService: Send + Sync + 'static {
    /// Only positively identified local solid-state storage permits parallel scans.
    /// Unsupported providers remain conservative without changing their path semantics.
    fn storage_kind(&self, _path: &Path) -> Result<StorageKind, FsError> {
        Ok(StorageKind::Unknown)
    }

    /// Returns available and total bytes for the filesystem containing `path`.
    ///
    /// # Errors
    /// Returns a contextual error for missing, inaccessible, or unsupported paths.
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError>;
}

/// Object-safe aggregate of the services required by the navigation engine.
/// Any provider implementing the three services automatically implements this trait.
pub trait FileSystem: DirectoryService + VolumeService + StorageService {}

impl<T: DirectoryService + VolumeService + StorageService + ?Sized> FileSystem for T {}
