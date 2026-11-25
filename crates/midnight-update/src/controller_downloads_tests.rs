use super::remove_other_downloads;
use std::fs;

#[test]
fn a_download_removes_the_folders_other_versions_left_and_keeps_its_own() {
    let private = tempfile::tempdir().unwrap();
    let downloads = private.path().join("download");
    let current = downloads.join("0.5.4");
    for version in ["0.5.2", "0.5.3", "0.5.4"] {
        let folder = downloads.join(version);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("archive.zip.part"), b"partial").unwrap();
    }
    let stray = downloads.join("notes.txt");
    fs::write(&stray, b"not a download").unwrap();

    remove_other_downloads(&current);

    assert!(!downloads.join("0.5.2").exists());
    assert!(!downloads.join("0.5.3").exists());
    assert_eq!(
        fs::read(current.join("archive.zip.part")).unwrap(),
        b"partial",
        "the current version's partial download is kept for resuming"
    );
    assert!(stray.exists(), "only folders are removed");
}

#[test]
fn a_first_download_has_nothing_to_remove() {
    let private = tempfile::tempdir().unwrap();
    remove_other_downloads(&private.path().join("download").join("0.5.4"));
    assert!(!private.path().join("download").exists());
}
