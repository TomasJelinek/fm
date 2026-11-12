#![cfg(target_os = "linux")]
use midnight_worker_platform::spool::PrivateSpoolDirectory;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
};

#[test]
fn spool_creation_is_anonymous_private_and_survives_directory_rename() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("private");
    let root = PrivateSpoolDirectory::open(&path).unwrap();
    let moved = temporary.path().join("moved");
    fs::rename(&path, &moved).unwrap();
    fs::create_dir(&path).unwrap();
    let mut spool = root.create().unwrap();
    spool.append(0, b"anonymous").unwrap();
    spool.finish().unwrap();
    assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 0);
    assert_eq!(fs::metadata(&moved).unwrap().mode() & 0o777, 0o700);
    let mut bytes = [0; 9];
    spool.read_at(0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"anonymous");
}

#[test]
fn spool_root_rejects_symlinks_and_nonprivate_permissions() {
    let temporary = tempfile::tempdir().unwrap();
    let outside = temporary.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o700)).unwrap();
    let link = temporary.path().join("linked");
    symlink(&outside, &link).unwrap();
    assert!(PrivateSpoolDirectory::open(&link).is_err());
    assert!(PrivateSpoolDirectory::open(&link.join("child")).is_err());
    assert!(!outside.join("child").exists());
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(PrivateSpoolDirectory::open(&outside).is_err());
}
