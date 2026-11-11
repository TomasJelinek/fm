#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]
#![cfg(target_os = "linux")]
//! Native linux adapter for the shared blocking filesystem service contracts.

use midnight_core::{
    Cancellation, DirectoryService, Entry, EntryMetadata, FsError, SpaceInfo, StorageService,
    Volume, VolumeService,
};
use midnight_platform_common::LocalServices;
use std::fs;
use std::path::{Path, PathBuf};
mod clipboard;
mod clipboard_text;
mod desktop;
mod directory_links;
mod editor;
mod file_clipboard;
mod file_drag_drop;
mod user_commands;
pub use clipboard::NativeClipboard;
pub use file_drag_drop::NativeFileDragDrop;
#[cfg(test)]
mod discovery_tests;
mod find;
mod fonts;
mod icons;
#[allow(unsafe_code)]
mod locale;
mod metadata;
mod mounts;
mod mutation;
mod open;
mod path_picker;
mod settings;
mod shell_menu;
mod size_directory;
#[allow(unsafe_code)]
mod sort_keys;
mod storage_kind;
mod transfer;
mod volume_information;
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
    fn resolve_size_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::SizeDirectoryTarget>, FsError> {
        size_directory::resolve(path, cancel)
    }
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.local.read_directory(path, cancel, &mut |mut entry| {
            directory_links::classify(&mut entry, cancel);
            !cancel.is_cancelled() && emit(entry)
        })
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        metadata::read(path)
    }
}

impl midnight_core::NavigationFallbackService for LinuxFileSystem {
    fn navigation_fallback_root(&self) -> Result<PathBuf, FsError> {
        Ok(PathBuf::from("/"))
    }
}

impl StorageService for LinuxFileSystem {
    fn volume_information(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<midnight_core::VolumeInformation, FsError> {
        volume_information::query(path, cancel)
    }
    fn storage_kind(&self, path: &Path) -> Result<midnight_core::StorageKind, FsError> {
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
