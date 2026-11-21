//! Permanent Delete offers each entry it cannot remove, typically a file that another
//! program keeps open without delete sharing: Retry repeats that step, Skip leaves the
//! entry with the folders that contain it and deletes the rest, and Cancel ends the
//! deletion. Read-only files and folders of the confirmed tree never ask.
use super::*;
use crate::test_fixtures::build_temp;
use midnight_core::{DeleteEntryFailure, DeleteFailureChoice};
use std::{fs, os::windows::fs::OpenOptionsExt};

/// Opens `path` as a program that does not let it be deleted while it is open.
fn lock(path: &Path) -> File {
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    fs::OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ_WRITE)
        .open(path)
        .unwrap()
}

fn prepare(path: &Path) -> Arc<dyn PreparedMutation> {
    WindowsFileSystem::new(None)
        .prepare(path, &Cancellation::default())
        .unwrap()
}

/// A permanent deletion with decisions, recording every failure offered.
fn delete_deciding(
    path: &Path,
    mut decide: impl FnMut(&DeleteEntryFailure) -> DeleteFailureChoice,
) -> (Result<DeleteCompletion, FsError>, Vec<DeleteEntryFailure>) {
    let mut offered = Vec::new();
    let result = prepare(path).delete_with_decisions(
        DeleteMode::Permanent,
        &Cancellation::default(),
        &mut |_| {},
        &mut |failure| {
            let choice = decide(&failure);
            offered.push(failure);
            choice
        },
    );
    (result, offered)
}

/// `tree` whose folder `nested` holds `locked.txt` beside entries that can be removed.
fn tree_with_a_locked_file(root: &Path) -> (PathBuf, PathBuf) {
    let tree = root.join("tree");
    let nested = tree.join("nested");
    fs::create_dir_all(nested.join("empty")).unwrap();
    fs::write(nested.join("locked.txt"), b"locked").unwrap();
    fs::write(nested.join("gone.txt"), b"gone").unwrap();
    fs::write(tree.join("other.txt"), b"other").unwrap();
    (tree, nested.join("locked.txt"))
}

fn set_read_only(path: &Path, read_only: bool) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(read_only);
    fs::set_permissions(path, permissions).unwrap();
}

fn read_only(path: &Path) -> bool {
    fs::metadata(path).unwrap().permissions().readonly()
}

#[test]
fn skip_keeps_the_entry_and_its_folders_and_removes_everything_else() {
    let temp = build_temp("delete-decision-");
    let (tree, locked) = tree_with_a_locked_file(temp.path());
    let held = lock(&locked);
    let (result, offered) = delete_deciding(&tree, |_| DeleteFailureChoice::Skip);
    drop(held);
    assert_eq!(offered.len(), 1, "{offered:?}");
    assert_eq!(offered[0].error.path, locked);
    assert!(offered[0].can_retry);
    assert!(
        offered[0].error.message.contains("(os error 32)"),
        "{:?}",
        offered[0]
    );
    let done = result.unwrap();
    assert_eq!(done.skipped, 1);
    assert!(done.residue.is_none());
    // The skipped file and the folders that contain it stay under their own names;
    // everything else was removed.
    assert_eq!(fs::read(&locked).unwrap(), b"locked");
    let nested = locked.parent().unwrap();
    assert!(!nested.join("gone.txt").exists());
    assert!(!nested.join("empty").exists());
    assert!(!tree.join("other.txt").exists());
}

#[test]
fn retry_once_the_file_is_closed_removes_everything() {
    let temp = build_temp("delete-decision-");
    let (tree, locked) = tree_with_a_locked_file(temp.path());
    let mut held = Some(lock(&locked));
    let mut asked = 0;
    let (result, offered) = delete_deciding(&tree, |failure| {
        // While the decision waits, the entry is under its own name, where the cause
        // can be fixed.
        assert_eq!(failure.error.path, locked);
        assert_eq!(fs::read(&locked).unwrap(), b"locked");
        asked += 1;
        // The first Retry finds the file still open and asks again.
        if asked == 2 {
            drop(held.take());
        }
        DeleteFailureChoice::Retry
    });
    assert_eq!(offered.len(), 2, "{offered:?}");
    let done = result.unwrap();
    assert_eq!(done.skipped, 0);
    assert!(!tree.exists());
}

#[test]
fn cancel_ends_the_deletion_and_keeps_what_is_left() {
    let temp = build_temp("delete-decision-");
    let (tree, locked) = tree_with_a_locked_file(temp.path());
    let held = lock(&locked);
    let (result, offered) = delete_deciding(&tree, |_| DeleteFailureChoice::Cancel);
    drop(held);
    assert_eq!(offered.len(), 1, "{offered:?}");
    assert_eq!(result.unwrap_err().kind, io::ErrorKind::Interrupted);
    assert_eq!(fs::read(&locked).unwrap(), b"locked");
}

#[test]
fn without_decisions_the_first_failure_ends_the_deletion() {
    let temp = build_temp("delete-decision-");
    let (tree, locked) = tree_with_a_locked_file(temp.path());
    let held = lock(&locked);
    let error = prepare(&tree)
        .delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
        .unwrap_err();
    drop(held);
    assert_eq!(error.path, locked);
    assert!(error.message.contains("(os error 32)"), "{error:?}");
    assert_eq!(fs::read(&locked).unwrap(), b"locked");
}

#[test]
fn a_folder_another_program_adds_to_is_offered_once_its_entries_are_done() {
    for skip in [false, true] {
        let temp = build_temp("delete-decision-");
        let tree = temp.path().join("tree");
        let nested = tree.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("gone.txt"), b"gone").unwrap();
        let late = nested.join("late.txt");
        let mut offered = Vec::new();
        let result = prepare(&tree).delete_with_decisions(
            DeleteMode::Permanent,
            &Cancellation::default(),
            &mut |removed| {
                // Another program adds an entry after the folder was listed.
                if removed.ends_with("gone.txt") {
                    fs::write(&late, b"late").unwrap();
                }
            },
            &mut |failure| {
                assert!(failure.can_retry);
                offered.push(failure.error.path);
                if skip {
                    DeleteFailureChoice::Skip
                } else {
                    // Retrying before the entry is gone asks again.
                    if offered.len() == 2 {
                        fs::remove_file(&late).unwrap();
                    }
                    DeleteFailureChoice::Retry
                }
            },
        );
        let done = result.unwrap();
        if skip {
            assert_eq!(offered, std::slice::from_ref(&nested));
            assert_eq!(done.skipped, 1);
            assert_eq!(fs::read(&late).unwrap(), b"late");
        } else {
            assert_eq!(offered, [nested.clone(), nested.clone()]);
            assert_eq!(done.skipped, 0);
            assert!(!tree.exists());
        }
    }
}

#[test]
fn read_only_files_and_folders_inside_the_tree_are_deleted_without_a_decision() {
    let temp = build_temp("delete-read-only-");
    let tree = temp.path().join("tree");
    let folder = tree.join("read-only folder");
    fs::create_dir_all(folder.join("deeper")).unwrap();
    for file in [
        tree.join("note.txt"),
        folder.join("file.txt"),
        folder.join("deeper").join("inner.txt"),
    ] {
        fs::write(&file, b"read-only").unwrap();
        set_read_only(&file, true);
    }
    // The selected folder itself too.
    for directory in [folder.join("deeper"), folder.clone(), tree.clone()] {
        set_read_only(&directory, true);
    }
    let mut removed = 0;
    let mut offered = Vec::new();
    let result = prepare(&tree).delete_with_decisions(
        DeleteMode::Permanent,
        &Cancellation::default(),
        &mut |_| removed += 1,
        &mut |failure| {
            offered.push(failure);
            DeleteFailureChoice::Cancel
        },
    );
    assert!(offered.is_empty(), "{offered:?}");
    let done = result.unwrap();
    assert_eq!(done.skipped, 0);
    assert_eq!(removed, 6);
    assert!(!tree.exists());
}

#[test]
fn read_only_folders_that_stay_keep_the_attribute() {
    let temp = build_temp("delete-read-only-");
    let tree = temp.path().join("tree");
    let kept = tree.join("kept");
    fs::create_dir_all(&kept).unwrap();
    let locked = kept.join("locked.txt");
    fs::write(&locked, b"locked").unwrap();
    let gone = kept.join("gone.txt");
    fs::write(&gone, b"gone").unwrap();
    set_read_only(&gone, true);
    set_read_only(&kept, true);
    set_read_only(&tree, true);
    let held = lock(&locked);
    let (result, offered) = delete_deciding(&tree, |_| DeleteFailureChoice::Skip);
    drop(held);
    let kept_read_only = read_only(&kept) && read_only(&tree);
    set_read_only(&kept, false);
    set_read_only(&tree, false);
    assert_eq!(offered.len(), 1, "{offered:?}");
    assert_eq!(offered[0].error.path, locked);
    assert_eq!(result.unwrap().skipped, 1);
    assert!(!gone.exists());
    // Deletion ignores Read-only without changing it, so what stays keeps it.
    assert!(kept_read_only);
    assert_eq!(fs::read(&locked).unwrap(), b"locked");
}
