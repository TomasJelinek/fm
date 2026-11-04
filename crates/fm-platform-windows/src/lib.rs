#![deny(unsafe_op_in_unsafe_fn)]
#![cfg(windows)]
//! Native windows adapter for the shared blocking filesystem service contracts.

use fm_core::{
    Cancellation, DirectoryService, Entry, EntryMetadata, FsError, SpaceInfo, StorageService,
    Volume, VolumeService,
};
use fm_platform_common::LocalServices;
use std::io;
use std::path::{Path, PathBuf};
mod desktop;
mod fonts;
mod icons;
mod open;
mod settings;
mod storage_kind;
mod terminal_process;
mod volume_details;
mod watch;

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
        self.local.read_directory(path, cancel, emit)
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        self.local.entry_metadata(path)
    }
}

impl StorageService for WindowsFileSystem {
    fn storage_kind(&self, path: &Path) -> Result<fm_core::StorageKind, FsError> {
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
