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
#[allow(unsafe_code)]
mod user_command_output;
pub use user_command_output::{run_and_wait, set_console_launcher};
mod user_commands;
pub use clipboard::NativeClipboard;
#[cfg(test)]
mod discovery_tests;
mod drive_names;
mod find;
mod fonts;
mod icon_picker;
mod icon_storage;
mod icons;
#[allow(unsafe_code)]
mod locale;
mod metadata;
mod mounts;
mod mutation;
mod open;
mod open_files;
pub use open_files::{MAX_OPEN_FILES, raise_open_file_limit, with_original_open_file_limit};
mod ownership;
mod path_picker;
mod settings;
mod shell_menu;
mod size_directory;
#[allow(unsafe_code)]
mod sort_keys;
mod storage_kind;
#[cfg(test)]
mod test_support;
mod transfer;
mod volume_information;
mod watch;

/// Native linux volume discovery combined with safe portable directory operations.
pub struct LinuxFileSystem {
    local: LocalServices,
    home: Option<PathBuf>,
    /// The one inotify instance every folder watch of this service shares.
    watches: std::sync::Arc<watch::Hub>,
}

// Requires the composed transfer_lease reply-owner handoff: no lease-worker file
// reference survives a successful reply. Native capability destruction is direct.
impl midnight_core::AuditedSynchronousTransferService for LinuxFileSystem {}

impl LinuxFileSystem {
    /// Creates an adapter with an optional Home root supplied by the application.
    /// The Home path is preserved verbatim and need not exist.
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            local: LocalServices::default(),
            home,
            watches: std::sync::Arc::default(),
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
    let mountinfo = Path::new("/proc/self/mountinfo");
    let input = fs::read(mountinfo)
        .map_err(|error| FsError::from_io("read mountinfo", mountinfo, error))?;
    Ok(mounts::volumes(
        &input,
        &mut volume_information::device_label,
    ))
}
