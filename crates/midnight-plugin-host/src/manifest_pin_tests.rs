//! A manifest pin accepts a moved change time only for the bytes it validated.
use super::ManifestPin;
use crate::{CapturedFile, RevisionPolicy};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[cfg(windows)]
#[path = "manifest_pin_windows_tests.rs"]
mod windows;

const MANIFEST: &[u8] = br#"{"schema":1,"version":"1"}"#;
const TAMPERED: &[u8] = br#"{"schema":1,"version":"2"}"#;
const BUDGET: u64 = 16 * 1024;

struct Fixture {
    _root: tempfile::TempDir,
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        // No extension: endpoint protection on some machines rescans fresh `.json` files and
        // would move the change time on its own.
        let path = root.path().join("manifest");
        fs::write(&path, MANIFEST).unwrap();
        Self { path, _root: root }
    }

    fn pin(&self) -> ManifestPin {
        ManifestPin::capture(&self.path, BUDGET).unwrap()
    }
}

/// Move the file's change time (NTFS ChangeTime, Linux ctime) and nothing else in its
/// revision: set and clear the read-only attribute (Windows) or the write bits (Linux).
fn move_change_time_only(path: &Path) {
    // Past the clock's granularity, so that the change time really moves.
    std::thread::sleep(Duration::from_millis(50));
    let writable = fs::metadata(path).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    fs::set_permissions(path, read_only).unwrap();
    fs::set_permissions(path, writable).unwrap();
}

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

fn set_write_time(path: &Path, time: SystemTime) {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
    assert_eq!(modified(path), time);
}

/// Overwrite the file's bytes in place (the same file and length), then set its write time
/// back, as whoever can write the file can.
fn overwrite_keeping_write_time(path: &Path, bytes: &[u8]) {
    let before = modified(path);
    std::thread::sleep(Duration::from_millis(50));
    let mut file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.write_all(bytes).unwrap();
    drop(file);
    set_write_time(path, before);
}

#[test]
fn a_moved_change_time_is_accepted_once_the_bytes_read_again_are_the_validated_ones() {
    let fixture = Fixture::new();
    let pin = fixture.pin();
    assert_eq!(pin.bytes(), MANIFEST);
    pin.revalidate().unwrap();
    assert_eq!(pin.rereads(), 0, "an unchanged revision is not read again");
    let exact = CapturedFile::capture(&fixture.path).unwrap();
    move_change_time_only(&fixture.path);
    assert!(exact.revalidate().is_err(), "the change time did not move");
    pin.revalidate().unwrap();
    assert_eq!(pin.rereads(), 1);
    pin.revalidate().unwrap();
    assert_eq!(
        pin.rereads(),
        1,
        "the verified fresh revision replaces the pinned one"
    );
    assert_eq!(pin.bytes(), MANIFEST);
}

#[test]
fn other_bytes_behind_a_restored_write_time_are_refused() {
    let fixture = Fixture::new();
    let pin = fixture.pin();
    let lenient =
        CapturedFile::capture_with(&fixture.path, RevisionPolicy::AcceptChangeTimeOnly).unwrap();
    overwrite_keeping_write_time(&fixture.path, TAMPERED);
    lenient
        .revalidate()
        .expect("identity, length and write time must be unchanged");
    let error = pin.revalidate().unwrap_err();
    assert!(
        error
            .0
            .starts_with("Source changed since capture (LiveValidated revision)"),
        "{error}"
    );
    assert!(
        error
            .0
            .ends_with("read again, its content differs from the validated content"),
        "{error}"
    );
    assert_eq!(pin.rereads(), 1);
    // The refusal lasts: the pin keeps the validated revision and bytes.
    assert!(pin.revalidate().is_err());
    assert_eq!(pin.bytes(), MANIFEST);
    // The pin proves content, not history: the validated bytes back in place pass again.
    overwrite_keeping_write_time(&fixture.path, MANIFEST);
    pin.revalidate().unwrap();
}

#[test]
fn a_moved_write_time_is_refused_without_reading_the_content_again() {
    let fixture = Fixture::new();
    let pin = fixture.pin();
    set_write_time(
        &fixture.path,
        modified(&fixture.path) + Duration::from_secs(2),
    );
    let error = pin.revalidate().unwrap_err();
    assert!(
        error.0.starts_with("Source changed since capture"),
        "{error}"
    );
    assert_eq!(pin.rereads(), 0);
}

#[test]
fn another_file_with_the_same_bytes_is_refused_without_reading_it() {
    let fixture = Fixture::new();
    let pin = fixture.pin();
    fs::rename(&fixture.path, fixture.path.with_extension("previous")).unwrap();
    fs::write(&fixture.path, MANIFEST).unwrap();
    assert!(pin.revalidate().is_err());
    assert_eq!(pin.rereads(), 0);
    fs::remove_file(&fixture.path).unwrap();
    assert!(pin.revalidate().is_err(), "a missing file is refused");
    assert_eq!(pin.rereads(), 0);
}

#[test]
fn capture_reads_the_whole_file_within_its_budget() {
    let fixture = Fixture::new();
    let error = ManifestPin::capture(&fixture.path, MANIFEST.len() as u64 - 1).unwrap_err();
    assert!(error.0.contains("budget"), "{error}");
    let pin = ManifestPin::capture(&fixture.path, MANIFEST.len() as u64).unwrap();
    assert_eq!(pin.bytes(), MANIFEST);
    let empty = fixture.path.with_extension("empty");
    fs::write(&empty, b"").unwrap();
    let pin = ManifestPin::capture(&empty, BUDGET).unwrap();
    assert!(pin.bytes().is_empty());
    pin.revalidate().unwrap();
    assert!(ManifestPin::capture(Path::new("relative"), BUDGET).is_err());
}
