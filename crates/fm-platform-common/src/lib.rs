#![forbid(unsafe_code)]
//! Portable, blocking local directory and capacity services shared by native adapters.
pub mod fonts;
pub mod settings;

use fm_core::{
    Cancellation, DirectoryService, Entry, EntryId, EntryKind, EntryMetadata, FsError, SpaceInfo,
    StorageService,
};
use std::{
    fs, io,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

/// Safe local operations preserving native paths and leaving volume policy to each OS.
/// Call service methods on background workers; cancellation cannot interrupt an OS call.
pub struct LocalServices {
    next_entry_id: AtomicU64,
}

impl Default for LocalServices {
    fn default() -> Self {
        Self {
            next_entry_id: AtomicU64::new(1),
        }
    }
}

impl LocalServices {
    /// Enumerate once and let a platform classify the cached, non-following file type.
    pub fn read_directory_with(
        &self,
        path: &Path,
        cancel: &Cancellation,
        classify: impl Fn(fs::FileType) -> EntryKind,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let mut directory =
            fs::read_dir(path).map_err(|error| FsError::from_io("read directory", path, error))?;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let Some(item) = directory.next() else {
                break;
            };
            let item = match item {
                Ok(item) => item,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(FsError::from_io("read directory entry", path, error)),
            };
            // DirEntry::file_type uses directory enumeration data where available;
            // it does not follow symlinks or fetch full per-entry metadata.
            let file_type = match item.file_type() {
                Ok(file_type) => file_type,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(FsError::from_io("read entry type", &item.path(), error)),
            };
            let kind = classify(file_type);
            if cancel.is_cancelled() {
                break;
            }
            let id = EntryId(self.next_entry_id.fetch_add(1, Ordering::Relaxed));
            if !emit(Entry::new(id, item.path(), kind)) {
                break;
            }
        }
        Ok(())
    }
}

impl DirectoryService for LocalServices {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.read_directory_with(path, cancel, classify, emit)
    }

    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        // Do not open targets: dangling links, FIFOs and devices are safe to inspect.
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| FsError::from_io("read metadata", path, error))?;
        Ok(EntryMetadata {
            size: metadata.is_file().then_some(metadata.len()),
            modified: metadata.modified().ok(),
            ..EntryMetadata::default()
        })
    }
}

fn classify(file_type: fs::FileType) -> EntryKind {
    if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

impl StorageService for LocalServices {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        // fs2's Windows implementation resolves a volume even for missing paths.
        // Match the Unix behavior and report the requested path as missing first.
        fs::symlink_metadata(path)
            .map_err(|error| FsError::from_io("read filesystem capacity", path, error))?;
        let stats = fs2::statvfs(path)
            .map_err(|error| FsError::from_io("read filesystem capacity", path, error))?;
        Ok(SpaceInfo {
            available: stats.available_space(),
            total: stats.total_space(),
        })
    }
}
