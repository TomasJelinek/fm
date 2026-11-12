//! Linux-specific path and special-file conformance, separate from the Windows adapter.
use crate::CapturedFile;
use std::{
    ffi::OsString,
    os::unix::{ffi::OsStringExt, fs::symlink},
};

pub(super) fn make_shared(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666)).unwrap();
}

#[test]
fn raw_byte_names_are_lossless_and_final_symlinks_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let path = root
        .path()
        .join(OsString::from_vec(b"raw-\xff.txt".to_vec()));
    std::fs::write(&path, b"data").unwrap();
    let capture = CapturedFile::capture(&path).unwrap();
    assert_eq!(capture.path(), path);
    assert_eq!(capture.open().unwrap().read_at(0, 4).unwrap(), b"data");
    let link = root.path().join("redirect");
    symlink(&path, &link).unwrap();
    assert!(CapturedFile::capture(&link).is_err());
}

#[test]
fn plugin_directory_symlink_escape_is_not_discovered() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("viewers")).unwrap();
    symlink(outside.path(), root.path().join("viewers/escape")).unwrap();
    let runtime = crate::Runtime::discover(root.path()).unwrap();
    assert!(runtime.catalog().is_empty());
    assert_eq!(runtime.diagnostics().len(), 1);
}

#[test]
fn native_device_files_are_refused() {
    assert!(CapturedFile::capture(std::path::Path::new("/dev/null")).is_err());
    assert!(CapturedFile::capture(std::path::Path::new("/dev/zero")).is_err());
}
