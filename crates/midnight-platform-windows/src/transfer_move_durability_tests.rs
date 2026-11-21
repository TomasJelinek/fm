//! WIN-6: a committed native Move saves the folders it changed (TRANSFERS.md, "Move
//! durability on Windows") and is confirmed; a folder it cannot save leaves the committed
//! Move unconfirmed.
use super::*;
use crate::test_fixtures::deny_to_everyone;
use midnight_core::TransferNativeMoveOutcome;
use std::{ffi::OsStr, fs::OpenOptions, os::windows::fs::OpenOptionsExt, path::Path};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY,
    FILE_SHARE_DELETE, FILE_SHARE_READ,
};

/// Moves `source` natively into `destination` as `name`; whether its durability is
/// confirmed.
fn native_move(source: &Path, destination: &Path, name: &str) -> bool {
    let cancel = Cancellation::default();
    let (_source, capture) = prepare_native(source, &cancel).unwrap();
    let directory = crate::WindowsFileSystem::new(None)
        .prepare_transfer_destination(destination, &cancel)
        .unwrap();
    match capture
        .try_native_move(directory.as_ref(), OsStr::new(name), None, &cancel)
        .unwrap()
    {
        TransferNativeMoveOutcome::Moved {
            destination: moved,
            recovery,
            durability_confirmed,
        } => {
            assert_eq!(moved, destination.join(name));
            assert!(recovery.is_empty());
            durability_confirmed
        }
        other => panic!("the native Move did not commit: {other:?}"),
    }
}

#[test]
fn a_native_move_of_a_file_a_folder_or_a_rename_is_confirmed_once_its_folders_are_saved() {
    let (_temp, parent) = fixture();
    let from = parent.join("from");
    let to = parent.join("to 文档");
    fs::create_dir(&from).unwrap();
    fs::create_dir(&to).unwrap();
    fs::write(from.join("file"), b"original").unwrap();
    // Empty: a scanner holding a just-written child would refuse the folder's rename
    // (TEST-04), which this test is not about.
    fs::create_dir(from.join("folder")).unwrap();
    assert!(native_move(&from.join("file"), &to, "file"));
    assert!(native_move(&from.join("folder"), &to, "folder"));
    // Within one folder: that folder is saved once.
    assert!(native_move(&to.join("file"), &to, "renamed"));
    assert_eq!(fs::read(to.join("renamed")).unwrap(), b"original");
    assert!(to.join("folder").is_dir());
    assert_eq!(fs::read_dir(&from).unwrap().count(), 0);
}

#[test]
fn a_native_move_out_of_a_folder_it_cannot_save_commits_but_stays_unconfirmed() {
    let (_temp, parent) = fixture();
    let to = parent.join("to");
    fs::create_dir(&to).unwrap();
    // Another program lists the source folder and denies write sharing; or the account may
    // add nothing to it (it may still remove its own entries). Either way the rename out
    // of it commits, but no handle that may flush the folder can be opened.
    for obstacle in ["held", "denied"] {
        let from = parent.join(obstacle);
        fs::create_dir(&from).unwrap();
        fs::write(from.join("first"), obstacle).unwrap();
        fs::write(from.join("second"), obstacle).unwrap();
        // The same Move without the obstacle is confirmed.
        assert!(native_move(
            &from.join("first"),
            &to,
            &format!("{obstacle} first")
        ));
        let other = match obstacle {
            "held" => Some(
                OpenOptions::new()
                    .access_mode(FILE_LIST_DIRECTORY)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                    .open(&from)
                    .unwrap(),
            ),
            _ => {
                // Not needed to remove the fixture, so the DACL is not restored.
                deny_to_everyone(&from, FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY).unwrap();
                None
            }
        };
        let name = format!("{obstacle} second");
        assert!(!native_move(&from.join("second"), &to, &name), "{obstacle}");
        assert_eq!(fs::read(to.join(&name)).unwrap(), obstacle.as_bytes());
        assert!(!from.join("second").exists());
        drop(other);
    }
}
