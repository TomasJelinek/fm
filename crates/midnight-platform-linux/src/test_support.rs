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

const ACCESS_ACL: &str = "system.posix_acl_access";

/// Lets another user (nobody) write to `path` through a POSIX access ACL:
/// `user::rwx user:<other>:rwx group::r-x mask::rwx other::r-x`, so the mode reads
/// 0775 like an ordinary private-group folder. Prints SKIP and returns false where
/// the filesystem has no POSIX ACLs.
pub(crate) fn share_through_access_acl(path: &Path, test: &str) -> bool {
    let caller = rustix::process::geteuid().as_raw();
    let other = if caller == 65534 { 65533 } else { 65534 };
    // The kernel's posix_acl_xattr format: version 2, then (tag, permissions, id)
    // entries in tag order; ids of the unnamed entries are undefined (u32::MAX).
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, permissions, id) in [
        (0x01u16, 7u16, u32::MAX),
        (0x02, 7, other),
        (0x04, 5, u32::MAX),
        (0x10, 7, u32::MAX),
        (0x20, 5, u32::MAX),
    ] {
        acl.extend_from_slice(&tag.to_le_bytes());
        acl.extend_from_slice(&permissions.to_le_bytes());
        acl.extend_from_slice(&id.to_le_bytes());
    }
    match rustix::fs::setxattr(path, ACCESS_ACL, &acl, rustix::fs::XattrFlags::empty()) {
        Ok(()) => true,
        Err(rustix::io::Errno::OPNOTSUPP) => {
            eprintln!("SKIP {test}: the temporary filesystem has no POSIX ACLs");
            false
        }
        Err(error) => panic!("setting a POSIX access ACL failed: {error}"),
    }
}

/// Removes the access ACL; the mode keeps the former mask as its group bits.
pub(crate) fn remove_access_acl(path: &Path) {
    rustix::fs::removexattr(path, ACCESS_ACL).unwrap();
}
