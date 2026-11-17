use super::*;
use crate::{WindowsFileSystem, test_fixtures::build_temp};
use midnight_core::{DeleteMode, MutationService};
use std::{cell::Cell, fs};

fn os(code: i32) -> io::Error {
    io::Error::from_raw_os_error(code)
}

#[test]
fn posix_disposition_falls_back_only_where_the_filesystem_lacks_it() {
    const CONFIRMED: FILE_DISPOSITION_INFO_EX_FLAGS =
        FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE;
    let classic = Cell::new(0);
    let run = |flags, extended: io::Result<()>, read_only: bool| {
        dispose(
            flags,
            || extended,
            || {
                classic.set(classic.get() + 1);
                Ok(())
            },
            || Ok(read_only),
        )
    };
    // Supported: the classic class is never used.
    assert!(run(CONFIRMED, Ok(()), false).is_ok());
    assert_eq!(classic.get(), 0);
    // FAT32/exFAT report the extended class as unsupported in one of three ways.
    for code in [1, 50, 87] {
        assert!(run(CONFIRMED, Err(os(code)), false).is_ok(), "{code}");
        assert!(run(0, Err(os(code)), true).is_ok(), "copy cleanup {code}");
    }
    assert_eq!(classic.get(), 6);
    // Other failures, such as a denied deletion, are never retried another way.
    let error = run(CONFIRMED, Err(os(5)), false).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(5));
    // The classic class cannot ignore Read-only: confirmed deletion reports it.
    let error = run(CONFIRMED, Err(os(87)), true).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("Read-only"), "{error}");
    assert_eq!(classic.get(), 6);
}

#[test]
fn permanent_deletion_skips_children_another_process_removed_first() {
    let temp = build_temp("delete-race-");
    let root = temp.path().join("tree");
    fs::create_dir(&root).unwrap();
    let names: Vec<_> = (0..6).map(|index| format!("child-{index}.txt")).collect();
    for name in &names {
        fs::write(root.join(name), b"x").unwrap();
    }
    let cancel = Cancellation::default();
    let prepared = WindowsFileSystem::new(None)
        .prepare(&root, &cancel)
        .unwrap();
    // After the first removal, another process deletes the remaining snapshotted
    // children: the deletion still reaches its requested end state.
    let mut reported = Vec::new();
    prepared
        .delete(DeleteMode::Permanent, &cancel, &mut |path| {
            if reported.is_empty() {
                for name in &names {
                    let _ = fs::remove_file(root.join(name));
                }
            }
            reported.push(path.to_owned());
        })
        .unwrap();
    assert!(!root.exists());
    assert_eq!(
        reported.len(),
        2,
        "one child and the directory: {reported:?}"
    );
    assert_eq!(reported.last(), Some(&root));
}

#[test]
fn file_identity_combines_volume_and_file_id() {
    let temp = build_temp("identity-");
    let path = temp.path().join("file");
    fs::write(&path, b"x").unwrap();
    let first = identity(&File::open(&path).unwrap()).unwrap();
    let second = identity(&File::open(&path).unwrap()).unwrap();
    assert!(first == second);
    let legacy = legacy_identity(&File::open(&path).unwrap()).unwrap();
    // On NTFS the 64-bit index is the low half of the 128-bit identifier.
    assert_eq!(legacy.id[..8], first.id[..8]);
    assert_eq!(legacy.volume as u32, first.volume as u32);
}
