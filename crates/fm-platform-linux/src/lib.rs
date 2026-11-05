#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]
//! Native linux adapter for the shared blocking filesystem service contracts.

use fm_core::{
    Cancellation, DirectoryService, Entry, EntryMetadata, FsError, SpaceInfo, StorageService,
    Volume, VolumeService,
};
use fm_platform_common::LocalServices;
use std::fs;
use std::path::{Path, PathBuf};
mod desktop;
mod fonts;
mod icons;
mod metadata;
mod mounts;
mod open;
mod settings;
mod storage_kind;
mod watch;
use mounts::mount_paths;

/// Native linux volume discovery combined with safe portable directory operations.
pub struct LinuxFileSystem {
    local: LocalServices,
    home: Option<PathBuf>,
}

impl LinuxFileSystem {
    /// Creates an adapter with an optional Home root supplied by the application.
    /// The Home path is preserved verbatim and need not exist.
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            local: LocalServices::default(),
            home,
        }
    }
}

impl DirectoryService for LinuxFileSystem {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.local.read_directory(path, cancel, emit)
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        metadata::read(path)
    }
}

impl StorageService for LinuxFileSystem {
    fn storage_kind(&self, path: &Path) -> Result<fm_core::StorageKind, FsError> {
        Ok(storage_kind::detect(path))
    }
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        self.local.space_info(path)
    }
}

impl VolumeService for LinuxFileSystem {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        let mut volumes = platform_volumes()?;
        if let Some(home) = &self.home {
            volumes.push(Volume {
                id: "home".into(),
                label: "Home".into(),
                root: home.clone(),
            });
        }
        Ok(volumes)
    }
}
/// Returns the native home environment path, or None when it is unset.
/// This preserves raw path data and does not require filesystem access.
pub fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn platform_volumes() -> Result<Vec<Volume>, FsError> {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let mountinfo = Path::new("/proc/self/mountinfo");
    let input = fs::read(mountinfo)
        .map_err(|error| FsError::from_io("read mountinfo", mountinfo, error))?;
    let mut roots = mount_paths(&input);
    if !roots.iter().any(|root| root == b"/") {
        roots.insert(0, b"/".to_vec());
    }
    Ok(roots
        .into_iter()
        .map(|bytes| {
            // Hex identity avoids collisions between distinct non-UTF-8 mount paths.
            let id = format!(
                "mount:{}",
                bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let root = PathBuf::from(OsString::from_vec(bytes));
            let label = root.display().to_string();
            Volume { id, label, root }
        })
        .collect())
}
