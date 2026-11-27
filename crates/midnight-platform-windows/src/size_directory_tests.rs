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
fn directory_identity_is_stable_and_distinguishes_directories() {
    let owned = crate::test_fixtures::build_temp("fixture-");
    let child = owned.path().join("child");
    fs::create_dir(&child).unwrap();
    let root = fs::canonicalize(owned.path()).unwrap();
    let child = fs::canonicalize(&child).unwrap();
    let first = identity(&open(&root), &root).unwrap();
    assert_eq!(first, identity(&open(&root), &root).unwrap());
    assert_ne!(first, identity(&open(&child), &child).unwrap());
    let legacy = native::legacy_identity(&open(&root)).unwrap();
    assert!(legacy.is_known());
    assert_ne!(
        legacy.id,
        native::legacy_identity(&open(&child)).unwrap().id
    );
}

/// A real ID or file index is used as reported. An all-zero one (the WebDAV redirector
/// and SMB servers that report index 0 for every entry) and a missing one are replaced
/// by a key of the canonical path: stable for one folder, distinct between folders, so
/// folder sizes, Properties and the folder tree never take a folder for its ancestor.
#[test]
fn an_unknown_identity_is_keyed_by_the_canonical_path() {
    let known = native::Identity {
        volume: 7,
        id: [9; 16],
    };
    let parent = Path::new(r"\\?\UNC\server\share\folder");
    let child = Path::new(r"\\?\UNC\server\share\folder\child");
    assert_eq!(
        choose_identity(Some(known), parent),
        SizeDirectoryIdentity {
            device: 7,
            object: [9; 16],
        }
    );
    let zero = native::Identity {
        volume: 7,
        id: [0; 16],
    };
    let first = choose_identity(Some(zero), parent);
    assert_eq!(first.device, 7);
    assert_ne!(first.object, [0; 16]);
    assert_eq!(first, choose_identity(Some(zero), parent));
    assert_ne!(first, choose_identity(Some(zero), child));
    let missing = choose_identity(None, parent);
    assert_eq!(missing, choose_identity(None, parent));
    assert_eq!(
        missing.device,
        choose_identity(None, child).device,
        "one share, one device"
    );
    assert_ne!(missing.object, choose_identity(None, child).object);
}

#[test]
fn every_missing_information_class_answer_is_unsupported() {
    for code in [1, 50, 87, 120, 124] {
        assert!(native::unsupported(&io::Error::from_raw_os_error(code)));
    }
    for code in [5, 64, 1231] {
        assert!(!native::unsupported(&io::Error::from_raw_os_error(code)));
    }
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

/// Folder identities on a real FAT32 or exFAT folder (`MIDNIGHT_TEST_FAT_DIRECTORY`,
/// read only) or network share (`MIDNIGHT_TEST_SHARE_DIRECTORY`): the folder and each
/// of its subfolders resolve to distinct, stable identities. Skipped without either.
#[test]
#[ignore = "opt-in: MIDNIGHT_TEST_FAT_DIRECTORY or MIDNIGHT_TEST_SHARE_DIRECTORY"]
fn real_volume_folders_have_distinct_size_identities() {
    for variable in [
        "MIDNIGHT_TEST_FAT_DIRECTORY",
        "MIDNIGHT_TEST_SHARE_DIRECTORY",
    ] {
        let Some(base) = std::env::var_os(variable) else {
            eprintln!("skipped: {variable} is not set");
            continue;
        };
        let cancel = Cancellation::default();
        let base = resolve(Path::new(&base), &cancel).unwrap().unwrap();
        let mut seen = vec![base.identity];
        for entry in fs::read_dir(&base.path).unwrap().take(64) {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_dir() {
                continue;
            }
            let target = resolve(&entry.path(), &cancel).unwrap().unwrap();
            assert_eq!(
                target.identity,
                resolve(&entry.path(), &cancel).unwrap().unwrap().identity
            );
            assert!(
                !seen.contains(&target.identity),
                "{}",
                entry.path().display()
            );
            seen.push(target.identity);
        }
        println!("{variable}: {} distinct folder identities", seen.len());
    }
}
