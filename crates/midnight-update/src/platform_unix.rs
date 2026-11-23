//! Unix file details of staging: the archive's execute bits.
use std::{fs, io, os::unix::fs::PermissionsExt, path::Path};

/// Gives a staged file mode 0755 when the archive marks it executable, 0644 otherwise.
pub(crate) fn set_executable(path: &Path, executable: bool) -> io::Result<()> {
    let mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// A busy file system or a process still holding a file: tried again for a moment.
pub(crate) fn is_transient_rename_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::ResourceBusy || error.kind() == io::ErrorKind::PermissionDenied
}

/// `$XDG_CACHE_HOME/midnight-explorer/updates` (or `~/.cache/...`), mode 0700.
pub(crate) fn private_folder() -> Option<std::path::PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    // Debug builds only: local end-to-end runs keep the profile below a test folder.
    let test_root = if cfg!(debug_assertions) {
        std::env::var_os("MIDNIGHT_TEST_DATA_ROOT")
    } else {
        None
    };
    let base = test_root
        .or_else(|| std::env::var_os("XDG_CACHE_HOME"))
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".cache"))
        })?;
    let folder = base.join("midnight-explorer").join("updates");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&folder)
        .ok()?;
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).ok()?;
    Some(folder)
}
