//! Linux-only spool tests, a module of the `spool` test executable.
use midnight_worker_platform::spool::PrivateSpoolDirectory;
use rustix::process::{DumpableBehavior, geteuid, set_dumpable_behavior};
use std::{
    fs, io,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
};

/// Open `path` as a spool root that differs from a valid one only in its owner: it
/// is reached without links and has no group or other permissions, so only the
/// owner check can refuse it.
fn assert_refused_by_the_owner_check(path: &Path) {
    let metadata = fs::metadata(path).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(
        metadata.mode() & 0o077,
        0,
        "{path:?} must pass the mode check"
    );
    assert_ne!(
        metadata.uid(),
        geteuid().as_raw(),
        "{path:?} must be foreign"
    );
    let error = PrivateSpoolDirectory::open(path).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
    // The OS opened the directory: the refusal is the check's own error.
    assert_eq!(error.raw_os_error(), None, "{error}");
}

#[test]
fn spool_root_owned_by_another_account_is_refused_by_the_owner_check() {
    const CHILD: &str = "MIDNIGHT_SPOOL_FOREIGN_OWNER_TEST";
    if std::env::var_os(CHILD).is_some() {
        // procfs reports a non-dumpable process's /proc/<pid> files as root's, and the
        // kernel still lets the process itself open its own fd directory (mode 0500).
        // The re-executed child keeps that attribute away from the other tests.
        set_dumpable_behavior(DumpableBehavior::NotDumpable).unwrap();
        assert_refused_by_the_owner_check(&PathBuf::from(format!(
            "/proc/{}/fd",
            std::process::id()
        )));
        return;
    }
    if geteuid().is_root() {
        // Root owns that procfs directory; give an owner-only directory to `nobody`.
        let temporary = tempfile::tempdir().unwrap();
        let foreign = temporary.path().join("foreign");
        fs::create_dir(&foreign).unwrap();
        fs::set_permissions(&foreign, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::chown(&foreign, Some(65_534), Some(65_534)).unwrap();
        assert_refused_by_the_owner_check(&foreign);
        return;
    }
    // The child runs only this test, selected by its module-qualified name.
    let module = module_path!().split_once("::").unwrap().1;
    let name =
        format!("{module}::spool_root_owned_by_another_account_is_refused_by_the_owner_check");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name.as_str()])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

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
