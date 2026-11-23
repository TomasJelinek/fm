//! Windows file details of staging: no execute bit to set.
use std::{io, path::Path};

/// Windows runs any `.exe`; there is no execute permission to give.
pub(crate) fn set_executable(_path: &Path, _executable: bool) -> io::Result<()> {
    Ok(())
}

/// Sharing and lock violations that endpoint protection or a closing process cause
/// for a moment; a rename is tried again for these.
pub(crate) fn is_transient_rename_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied
        || matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

/// `%LOCALAPPDATA%\Midnight Explorer\updates`, which inherits the profile's
/// owner-only permissions.
pub(crate) fn private_folder() -> Option<std::path::PathBuf> {
    // Debug builds only: local end-to-end runs keep the profile below a test folder
    // (as the settings do); release builds ignore the variable.
    let test_root = if cfg!(debug_assertions) {
        std::env::var_os("MIDNIGHT_TEST_DATA_ROOT")
    } else {
        None
    };
    let base = std::path::PathBuf::from(test_root.or_else(|| std::env::var_os("LOCALAPPDATA"))?);
    if !base.is_absolute() {
        return None;
    }
    let folder = base.join("Midnight Explorer").join("updates");
    std::fs::create_dir_all(&folder).ok()?;
    Some(folder)
}
