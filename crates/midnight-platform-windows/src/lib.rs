#![deny(unsafe_op_in_unsafe_fn)]
#![cfg(windows)]
//! Native windows adapter for the shared blocking filesystem service contracts.

use midnight_core::{
    Cancellation, DirectoryService, Entry, EntryMetadata, FsError, SpaceInfo, StorageService,
    Volume, VolumeService,
};
use midnight_platform_common::LocalServices;
use std::io;
use std::path::{Path, PathBuf};
mod clipboard;
mod desktop;
mod editor;
mod file_clipboard;
mod file_drag_drop;
mod user_command_environment;
#[cfg(test)]
mod user_command_terminal_tests;
mod user_commands;
pub use clipboard::NativeClipboard;
mod find;
mod fonts;
mod icons;
mod listing;
mod locale;
mod locations;
mod metadata;
mod mutation;
mod network;
mod onedrive_locations;
mod open;
mod settings;
mod shell_menu;
mod sort_keys;
mod storage_kind;
mod terminal_process;
mod transfer;
mod volume_details;
mod watch;
mod window_geometry;
pub use window_geometry::monitor_work_area;

/// Native windows volume discovery combined with safe portable directory operations.
pub struct WindowsFileSystem {
    local: LocalServices,
    home: Option<PathBuf>,
}

impl WindowsFileSystem {
    /// Creates an adapter with an optional Home root supplied by the application.
    /// The Home path is preserved verbatim and need not exist.
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            local: LocalServices::default(),
            home,
        }
    }
}

impl DirectoryService for WindowsFileSystem {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        // Windows DirEntry metadata is cached enumeration data, without another OS call.
        self.local.read_directory_with_metadata(
            path,
            cancel,
            listing::classify,
            |item| {
                item.metadata()
                    .ok()
                    .map(|value| metadata::from_native(&value))
            },
            emit,
        )
    }
    fn listing_metadata_complete(&self, entry: &Entry) -> bool {
        entry.metadata.native_attributes.is_some()
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        metadata::read(path)
    }
}

impl StorageService for WindowsFileSystem {
    fn storage_kind(&self, path: &Path) -> Result<midnight_core::StorageKind, FsError> {
        Ok(storage_kind::detect(path))
    }
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        volume_details::space(path)
    }
}

impl VolumeService for WindowsFileSystem {
    fn volume_label(&self, path: &Path) -> Result<Option<std::ffi::OsString>, FsError> {
        volume_details::label(path)
    }

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
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

fn platform_volumes() -> Result<Vec<Volume>, FsError> {
    // SAFETY: GetLogicalDrives takes no pointers or arguments and returns a bitmask.
    let mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
    if mask == 0 {
        return Err(FsError::from_io(
            "list logical drives",
            Path::new(""),
            io::Error::last_os_error(),
        ));
    }
    Ok(drive_roots(mask)
        .into_iter()
        .map(|root| {
            let label = root.display().to_string();
            Volume {
                id: format!("drive:{label}"),
                label,
                root,
            }
        })
        .collect())
}

fn drive_roots(mask: u32) -> Vec<PathBuf> {
    (0..26)
        .filter(|bit| mask & (1 << bit) != 0)
        .map(|bit| PathBuf::from(format!("{}:\\", char::from(b'A' + bit as u8))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drive_mask_decodes_letter_positions_and_ignores_unused_bits() {
        assert_eq!(
            drive_roots((1 << 2) | (1 << 25) | (1 << 31)),
            vec![PathBuf::from("C:\\"), PathBuf::from("Z:\\")]
        );
        assert!(drive_roots(0).is_empty());
    }
}
