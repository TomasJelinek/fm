//! Windows-specific native identity coverage.
use crate::CapturedFile;
#[path = "tests_windows_capture.rs"]
mod capture;

pub(super) fn make_shared(path: &std::path::Path) {
    crate::trust::fixture_acl(path, "D:P(A;;FA;;;WD)").unwrap();
}

#[test]
fn replacement_with_same_size_and_timestamp_is_still_a_different_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("original");
    std::fs::write(&path, b"original").unwrap();
    let capture = CapturedFile::capture(&path).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::rename(&path, root.path().join("renamed")).unwrap();
    std::fs::write(&path, b"original").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert!(capture.open().is_err());
}
