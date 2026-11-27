//! The prepared identity's manifest pin under metadata-only and restored-write-time changes.
use super::*;
use midnight_plugin_host::{CapturedFile, RevisionPolicy};
use std::io::Write;

fn manifest_path(fixture: &Fixture) -> PathBuf {
    fixture.provider.package_dir().join("image-viewer.json")
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

/// Overwrite the file's bytes in place (the same file and length), then set its write time
/// back, as someone who can write the file can.
fn overwrite_keeping_write_time(path: &Path, bytes: &[u8]) {
    let modified = fs::metadata(path).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let mut file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.write_all(bytes).unwrap();
    drop(file);
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), modified);
}

/// Endpoint protection or a backup agent can update a manifest's metadata (an alternate
/// data stream, its attributes or ACL), which moves its change time alone. The prepared
/// identity then reads the manifest again, finds the bytes it validated and stays valid.
#[test]
fn a_manifest_whose_change_time_alone_moved_still_validates_the_prepared_identity() {
    let fixture = Fixture::new();
    let identity = ProviderIdentity::capture(&fixture.provider).unwrap();
    let path = manifest_path(&fixture);
    let exact = CapturedFile::capture(&path).unwrap();
    move_change_time_only(&path);
    assert!(exact.revalidate().is_err(), "the change time did not move");
    identity.validate_for(&fixture.provider).unwrap();
    identity.validate_for(&fixture.provider).unwrap();
}

/// A manifest changed in place with its write time set back differs from the validated one
/// only in its change time; the bytes read again differ, so the identity stays invalid.
#[test]
fn changed_manifest_bytes_behind_a_restored_write_time_invalidate_the_prepared_identity() {
    let fixture = Fixture::new();
    let identity = ProviderIdentity::capture(&fixture.provider).unwrap();
    let path = manifest_path(&fixture);
    let lenient = CapturedFile::capture_with(&path, RevisionPolicy::AcceptChangeTimeOnly).unwrap();
    let original = String::from_utf8(fs::read(&path).unwrap()).unwrap();
    let tampered = original.replace(r#""version":"1""#, r#""version":"2""#);
    assert_ne!(tampered, original);
    assert_eq!(tampered.len(), original.len());
    overwrite_keeping_write_time(&path, tampered.as_bytes());
    lenient
        .revalidate()
        .expect("identity, length and write time must be unchanged");
    let error = identity.validate_for(&fixture.provider).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Source changed since capture"),
        "{error}"
    );
    assert!(identity.validate_for(&fixture.provider).is_err());
}
