//! WIN-6: a committed native Move saves the folders it changed (transfers.md, "Move
//! durability on Windows") and is confirmed; a folder it cannot save leaves the committed
//! Move unconfirmed. A removal by delete disposition is confirmed once its name is gone.
use super::*;
use crate::test_fixtures::{create_junction, deny_to_everyone};
use midnight_core::{
    CopyLinkPolicy, PreparedTransferMove, PreparedTransferMoveEntry, TransferDirectoryLinkRemoval,
    TransferDurabilityRound, TransferMoveFinalizeOutcome, TransferNativeMoveOutcome,
    TransferReplacementPolicy, TransferSourceRemoval,
};
use std::{ffi::OsStr, fs::OpenOptions, io::Read, os::windows::fs::OpenOptionsExt, path::Path};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

/// The folder flushes this test's thread has requested so far.
fn flushes() -> usize {
    super::super::folder_flush::FLUSHES.with(std::cell::Cell::get)
}

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

#[test]
fn deferred_renames_save_their_folders_once_per_round_and_not_before() {
    let (_temp, parent) = fixture();
    let from = parent.join("from");
    let to = parent.join("to");
    fs::create_dir(&from).unwrap();
    fs::create_dir(&to).unwrap();
    for name in ["a", "b", "c", "d"] {
        fs::write(from.join(name), name).unwrap();
    }
    let cancel = Cancellation::default();
    let directory = crate::WindowsFileSystem::new(None)
        .prepare_transfer_destination(&to, &cancel)
        .unwrap();
    // Saved at once (as Rename's replacement does): both folders for each rename.
    let before = flushes();
    assert!(native_move(&from.join("a"), &to, "a"));
    assert_eq!(flushes() - before, 2);
    // Deferred (as a Move does): nothing until the engine's round, which saves the source
    // and the destination folder once for all three renames.
    let before = flushes();
    let confirmations: Vec<_> = ["b", "c", "d"]
        .into_iter()
        .map(|name| {
            let (_source, capture) = prepare_native(&from.join(name), &cancel).unwrap();
            let (outcome, confirmation) = capture
                .try_native_move_deferred(directory.as_ref(), OsStr::new(name), None, &cancel)
                .unwrap();
            assert!(matches!(
                outcome,
                TransferNativeMoveOutcome::Moved {
                    durability_confirmed: false,
                    ..
                }
            ));
            confirmation.expect("a deferred rename has a confirmation")
        })
        .collect();
    assert_eq!(flushes(), before, "no folder is saved before the round");
    let mut round = TransferDurabilityRound::default();
    let confirmed: Vec<_> = confirmations
        .into_iter()
        .map(|confirmation| confirmation.confirm(&mut round))
        .collect();
    assert_eq!(confirmed, [true; 3]);
    assert_eq!(flushes() - before, 2);
    assert_eq!(fs::read_dir(&from).unwrap().count(), 0);
    assert_eq!(fs::read(to.join("d")).unwrap(), b"d");
}

#[test]
fn a_deferred_replacement_saves_the_folder_keeping_the_original_outside_its_turn() {
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    let to = parent.join("to");
    fs::create_dir(&to).unwrap();
    fs::write(&source, b"new").unwrap();
    fs::write(to.join("target"), b"old").unwrap();
    let cancel = Cancellation::default();
    let directory = crate::WindowsFileSystem::new(None)
        .prepare_transfer_destination(&to, &cancel)
        .unwrap();
    let consent = directory
        .prepare_target(OsStr::new("target"), &cancel)
        .unwrap()
        .unwrap();
    let (_source, capture) = prepare_native(&source, &cancel).unwrap();
    let before = flushes();
    let (outcome, confirmation) = capture
        .try_native_move_deferred(
            directory.as_ref(),
            OsStr::new("target"),
            Some((consent, TransferReplacementPolicy::RecoverableCurrentEntry)),
            &cancel,
        )
        .unwrap();
    let TransferNativeMoveOutcome::Moved { recovery, .. } = outcome else {
        panic!("the replacement did not commit: {outcome:?}");
    };
    // Not even the private folder holding the original is flushed inside the rename's turn.
    assert_eq!(flushes(), before);
    assert!(
        confirmation
            .unwrap()
            .confirm(&mut TransferDurabilityRound::default())
    );
    // That folder, the destination and the source folder.
    assert_eq!(flushes() - before, 3);
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"old");
    assert_eq!(fs::read(to.join("target")).unwrap(), b"new");
}

/// A verified (cross-volume) Move removes its staged source by delete disposition; the
/// removal is confirmed once the source's handle (shared by the source, its capture and its
/// finalizer) closes, also while another program still reads the file, and not before.
#[test]
fn a_verified_file_removal_is_confirmed_once_the_source_handle_closes() {
    for keep_source in [false, true] {
        let (_temp, parent) = fixture();
        let path = parent.join("removed 文档");
        fs::write(&path, b"verified original").unwrap();
        let mut reader = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&path)
            .unwrap();
        let cancel = Cancellation::default();
        let (source, capture) = prepare_native(&path, &cancel).unwrap();
        let TransferMoveCaptureOutcome::Captured { recovery, .. } =
            capture.capture(&cancel).unwrap()
        else {
            panic!("capture failed")
        };
        let mut finalizer = capture.prepare_finalization(&cancel).unwrap();
        assert!(matches!(
            finalizer.finalize(&cancel),
            TransferMoveFinalizeOutcome::Removed {
                state: TransferSourceRemoval::DeletePending,
                durability_confirmed: false,
                ..
            }
        ));
        let confirmation = finalizer.take_removal_confirmation().unwrap();
        assert!(
            finalizer.take_removal_confirmation().is_none(),
            "taken once"
        );
        let cleanup = capture.take_stage_cleanup().unwrap();
        drop(finalizer);
        drop(capture);
        // Kept, the source still holds the deleting handle: the name is still there.
        let held = keep_source.then_some(source);
        assert_eq!(
            confirmation.confirm(&mut TransferDurabilityRound::default()),
            !keep_source
        );
        drop(held);
        cleanup.cleanup().unwrap();
        assert!(!recovery.path.exists() && !path.exists());
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"verified original");
    }
}

/// The folder a Move emptied is removed by delete disposition; the removal is confirmed
/// once the folder's handle (shared by the traversal's source and its capture) closes.
#[test]
fn an_emptied_folder_removal_is_confirmed_once_the_folder_handle_closes() {
    for keep_source in [false, true] {
        let (_temp, parent) = fixture();
        let folder = parent.join("emptied 文档");
        let to = parent.join("to");
        fs::create_dir(&folder).unwrap();
        fs::create_dir(&to).unwrap();
        fs::write(folder.join("child"), b"moved").unwrap();
        let filesystem = crate::WindowsFileSystem::new(None);
        let cancel = Cancellation::default();
        let mut prepared = filesystem.prepare_move_source(&folder, &cancel).unwrap();
        let destination = filesystem.prepare_move_destination(&to, &cancel).unwrap();
        let child = prepared.source.next_move_child(&cancel).unwrap().unwrap();
        assert!(matches!(
            child
                .capture
                .try_native_move(destination.as_ref(), OsStr::new("child"), None, &cancel)
                .unwrap(),
            TransferNativeMoveOutcome::Moved { .. }
        ));
        drop(child);
        assert!(prepared.source.next_move_child(&cancel).unwrap().is_none());
        let removal = prepared.capture.remove_empty_directory(&cancel).unwrap();
        assert_eq!(
            (removal.state, removal.durability_confirmed),
            (TransferSourceRemoval::DeletePending, false)
        );
        let confirmation = prepared.capture.take_removal_confirmation().unwrap();
        let PreparedTransferMove { source, capture } = prepared;
        drop(capture);
        let held = keep_source.then_some(source);
        assert_eq!(
            confirmation.confirm(&mut TransferDurabilityRound::default()),
            !keep_source
        );
        drop(held);
        assert!(!folder.exists());
        assert_eq!(fs::read(to.join("child")).unwrap(), b"moved");
    }
}

/// A directory link's original is removed through the capture's own deleting handle, which
/// the removal closes: the link is gone, and its removal confirmed, when it returns. A
/// folder that cannot be saved (another program holds it without write sharing) leaves the
/// removal unconfirmed and pending, the link gone all the same.
#[test]
fn a_directory_link_original_is_gone_and_confirmed_when_its_removal_returns() {
    for held_folder in [false, true] {
        let (_temp, parent) = fixture();
        let target = parent.join("target");
        let link = parent.join("link 文档");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("kept"), b"referent").unwrap();
        fs::create_dir(&link).unwrap();
        create_junction(&link, &target).unwrap();
        let cancel = Cancellation::default();
        let PreparedTransferMoveEntry::DirectoryLink { source, capture } =
            crate::WindowsFileSystem::new(None)
                .prepare_move_source_with_links(
                    &link,
                    CopyLinkPolicy::FollowDirectoryContents,
                    &cancel,
                )
                .unwrap()
        else {
            panic!("expected the original-only directory link capture");
        };
        let other = held_folder.then(|| {
            OpenOptions::new()
                .access_mode(FILE_LIST_DIRECTORY)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(&parent)
                .unwrap()
        });
        let TransferDirectoryLinkRemoval::Removed {
            state,
            durability_confirmed,
            recovery,
            error: None,
        } = capture.remove_original(&cancel).unwrap()
        else {
            panic!("the link was not removed");
        };
        assert!(recovery.is_empty());
        let expected = if held_folder {
            (TransferSourceRemoval::DeletePending, false)
        } else {
            (TransferSourceRemoval::Unlinked, true)
        };
        assert_eq!((state, durability_confirmed), expected);
        // Gone while the capture and its source are still held; the referent is untouched.
        assert!(fs::symlink_metadata(&link).is_err());
        assert!(capture.revalidate(&cancel).is_err());
        assert_eq!(fs::read(target.join("kept")).unwrap(), b"referent");
        drop((source, capture, other));
    }
}
