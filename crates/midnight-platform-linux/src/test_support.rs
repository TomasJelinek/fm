//! Unit-test fixtures. Every directory is unique, private and removed on drop, so
//! runs never share or leave fixture parents; CI points TMPDIR below `_build`.
use std::path::Path;

/// A temporary directory below the configured temporary directory.
pub(crate) fn temporary(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new().prefix(prefix).tempdir().unwrap()
}

/// A temporary directory on a local ext filesystem, which recoverable replacement
/// and verified Move removal qualify; elsewhere (Btrfs, XFS, tmpfs) prints SKIP.
pub(crate) fn ext_temporary(prefix: &str, test: &str) -> Option<tempfile::TempDir> {
    let fixture = temporary(prefix);
    if rustix::fs::statfs(fixture.path()).is_ok_and(|stat| stat.f_type == 0xef53) {
        Some(fixture)
    } else {
        eprintln!("SKIP {test}: the temporary directory is not on a local ext filesystem");
        None
    }
}

/// A private directory on the tmpfs at `/dev/shm`, the second local filesystem
/// for cross-device fixtures; prints SKIP when it is unavailable.
pub(crate) fn shm_temporary(prefix: &str, test: &str) -> Option<tempfile::TempDir> {
    match tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in("/dev/shm")
    {
        Ok(fixture) => Some(fixture),
        Err(error) => {
            eprintln!("SKIP {test}: /dev/shm is unavailable: {error}");
            None
        }
    }
}

/// A directory below the workspace `_build/tmp`, for fixtures that execute files
/// (a temporary directory may be mounted noexec).
pub(crate) fn workspace_temporary(prefix: &str) -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(root.canonicalize().unwrap())
        .unwrap()
}
