use super::*;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

fn fixture() -> tempfile::TempDir {
    let root = std::env::temp_dir().join("midnight-native-drag/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

#[test]
fn native_drop_device_recognizes_same_mount_without_read_permission_or_link_following() {
    let temp = fixture();
    let file = temp.path().join("unreadable");
    fs::write(&file, b"untouched content").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o0)).unwrap();
    let link = temp.path().join("foreign-target");
    symlink("/dev/shm", &link).unwrap();
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let device = service.native_drop_device(temp.path(), &cancel).unwrap();
    assert!(device.is_some(), "a local mount must be recognized");
    assert_eq!(service.native_drop_device(&file, &cancel).unwrap(), device);
    assert_eq!(service.native_drop_device(&link, &cancel).unwrap(), device);
    assert_ne!(
        service
            .native_drop_device(Path::new("/dev/shm"), &cancel)
            .unwrap(),
        device
    );
    assert!(file.exists() && link.is_symlink());
    assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0);
}

#[test]
fn native_drop_device_obeys_cancellation_and_reports_missing_paths() {
    let temp = fixture();
    let missing = temp.path().join("missing");
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    assert_eq!(
        service
            .native_drop_device(&missing, &cancel)
            .unwrap_err()
            .kind,
        io::ErrorKind::NotFound
    );
    cancel.cancel();
    assert_eq!(
        service
            .native_drop_device(&missing, &cancel)
            .unwrap_err()
            .kind,
        io::ErrorKind::Interrupted
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}
