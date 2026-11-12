use super::*;

fn open(path: &Path) -> fs::File {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap()
}

#[test]
fn legacy_directory_identity_is_stable_and_distinguishes_directories() {
    let owned = tempfile::tempdir().unwrap();
    let child = owned.path().join("child");
    fs::create_dir(&child).unwrap();
    let first = legacy_identity(&open(owned.path())).unwrap();
    assert_eq!(first, legacy_identity(&open(owned.path())).unwrap());
    assert_ne!(first, legacy_identity(&open(&child)).unwrap());
    for code in [1, 50, 87] {
        assert!(unsupported(&io::Error::from_raw_os_error(code)));
    }
    assert!(!unsupported(&io::Error::from_raw_os_error(5)));
}

#[test]
#[ignore = "read-only installed-system junction probe; invoke explicitly"]
fn legacy_documents_and_settings_resolves_and_enumerates_one_entry() {
    let path = Path::new("C:\\Documents and Settings");
    let target = resolve(path, &Cancellation::default()).unwrap().unwrap();
    let first = fs::read_dir(&target.path).unwrap().next();
    if let Some(first) = first {
        first.unwrap();
    }
}
