use crate::TrustedExecutablePin;
use std::{
    fs::{self, File, Permissions},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
};

const ORIGINAL: &[u8] = b"worker original";

struct Fixture {
    _root: tempfile::TempDir,
    package: PathBuf,
    executable: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("worker-package");
        fs::create_dir(&package).unwrap();
        fs::set_permissions(&package, Permissions::from_mode(0o700)).unwrap();
        let executable = package.join("worker");
        fs::write(&executable, ORIGINAL).unwrap();
        fs::set_permissions(&executable, Permissions::from_mode(0o600)).unwrap();
        Self {
            package: package.canonicalize().unwrap(),
            executable: executable.canonicalize().unwrap(),
            _root: root,
        }
    }

    fn capture(&self) -> TrustedExecutablePin {
        TrustedExecutablePin::capture(&self.package, &self.executable).unwrap()
    }
}

#[test]
fn unchanged_trusted_executable_validates_and_releases() {
    let fixture = Fixture::new();
    let pin = fixture.capture();
    pin.revalidate().unwrap();
    pin.release_on_worker().unwrap();
    fs::remove_file(&fixture.executable).unwrap();
}

#[test]
fn non_windows_executable_capture_rejects_a_changed_content_revision() {
    let fixture = Fixture::new();
    let pin = fixture.capture();
    fs::write(&fixture.executable, b"a different content length").unwrap();
    assert!(pin.revalidate().is_err());
    pin.release_on_worker().unwrap();
}

#[test]
fn non_windows_capture_rejects_replacement_with_matching_content_and_write_time() {
    let fixture = Fixture::new();
    let original = File::open(&fixture.executable).unwrap();
    let modified = original.metadata().unwrap().modified().unwrap();
    let pin = fixture.capture();
    let retired = fixture.package.join("retired-worker");
    fs::rename(&fixture.executable, &retired).unwrap();
    fs::write(&fixture.executable, ORIGINAL).unwrap();
    fs::set_permissions(&fixture.executable, Permissions::from_mode(0o600)).unwrap();
    File::options()
        .write(true)
        .open(&fixture.executable)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert_eq!(fs::read(&fixture.executable).unwrap(), ORIGINAL);
    assert_eq!(
        fs::metadata(&fixture.executable)
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    assert!(pin.revalidate().is_err());
    pin.release_on_worker().unwrap();
    drop(original);
    fixture.capture().release_on_worker().unwrap();
}
