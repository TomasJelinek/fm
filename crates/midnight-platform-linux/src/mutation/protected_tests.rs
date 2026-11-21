//! Write-protected folders inside a confirmed tree, the folder that contains the
//! selection, links, and decisions about entries that cannot be removed.
use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

/// Folder permissions do not bind the superuser.
fn privileged() -> bool {
    let root = rustix::process::geteuid().is_root();
    if root {
        eprintln!("SKIP: permissions do not apply to the superuser");
    }
    root
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn prepared(path: &Path) -> Prepared {
    Prepared::new(path, None, &Cancellation::default()).unwrap()
}

/// A permanent deletion with decisions, recording every failure offered and the
/// paths that existed while each waited.
fn delete_deciding(
    path: &Path,
    mut decide: impl FnMut(&DeleteEntryFailure) -> DeleteFailureChoice,
) -> (Result<DeleteCompletion, FsError>, Vec<DeleteEntryFailure>) {
    let mut offered = Vec::new();
    let result = prepared(path).delete_with_decisions(
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

#[test]
fn write_protected_folders_inside_the_tree_are_deleted_without_a_decision() {
    if privileged() {
        return;
    }
    for decisions in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let tree = fixture.path().join("tree");
        let locked = tree.join("readonly").join("locked-folder");
        fs::create_dir_all(locked.join("deeper")).unwrap();
        fs::write(locked.join("file.txt"), b"inside").unwrap();
        fs::write(locked.join("deeper").join("inner.txt"), b"deeper").unwrap();
        // A read-only file needs nothing: only its folder decides.
        fs::write(tree.join("readonly").join("note.txt"), b"beside").unwrap();
        chmod(&tree.join("readonly").join("note.txt"), 0o444);
        // Without search permission too, and the selected folder itself.
        chmod(&locked.join("deeper"), 0o444);
        chmod(&locked, 0o555);
        chmod(&tree.join("readonly"), 0o500);
        chmod(&tree, 0o555);
        let mut removed = 0;
        let done = if decisions {
            let (result, offered) = delete_deciding(&tree, |_| DeleteFailureChoice::Cancel);
            assert!(offered.is_empty(), "{offered:?}");
            result
        } else {
            prepared(&tree).delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {
                removed += 1
            })
        }
        .unwrap();
        assert_eq!(done.skipped, 0);
        assert!(done.residue.is_none());
        assert!(!tree.exists());
        if !decisions {
            assert_eq!(removed, 7);
        }
    }
}

#[test]
fn a_write_protected_containing_folder_is_named_and_never_changed() {
    if privileged() {
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let parent = fixture.path().join("protected");
    let item = parent.join("item");
    fs::create_dir_all(&item).unwrap();
    fs::write(item.join("file.txt"), b"kept").unwrap();
    chmod(&parent, 0o555);
    let (result, offered) = delete_deciding(&item, |_| DeleteFailureChoice::Skip);
    let failure = result.unwrap_err();
    assert!(offered.is_empty(), "{offered:?}");
    assert_eq!(failure.kind, ErrorKind::PermissionDenied);
    assert_eq!(failure.path, parent);
    assert!(failure.message.contains("write-protected"), "{failure:?}");
    assert_eq!(mode(&parent), 0o555);
    assert_eq!(fs::read(item.join("file.txt")).unwrap(), b"kept");
    chmod(&parent, 0o755);
}

#[test]
fn a_link_to_a_write_protected_folder_elsewhere_never_changes_that_folder() {
    if privileged() {
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let outside = fixture.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("file.txt"), b"elsewhere").unwrap();
    chmod(&outside, 0o555);
    let tree = fixture.path().join("tree");
    fs::create_dir(&tree).unwrap();
    symlink(&outside, tree.join("link")).unwrap();
    chmod(&tree, 0o555);
    let (result, offered) = delete_deciding(&tree, |_| DeleteFailureChoice::Cancel);
    assert!(offered.is_empty(), "{offered:?}");
    assert_eq!(result.unwrap().skipped, 0);
    assert!(!tree.exists());
    assert_eq!(mode(&outside), 0o555);
    assert_eq!(fs::read(outside.join("file.txt")).unwrap(), b"elsewhere");
    chmod(&outside, 0o755);
}

/// A tree whose write-protected folder holds an unreadable folder, the one entry that
/// cannot be removed, beside entries that can.
fn tree_with_an_unreadable_folder(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let tree = root.join("tree");
    let protected = tree.join("protected");
    let unreadable = protected.join("unreadable");
    fs::create_dir_all(&unreadable).unwrap();
    fs::write(unreadable.join("hidden.txt"), b"hidden").unwrap();
    fs::write(protected.join("gone.txt"), b"gone").unwrap();
    fs::write(tree.join("other.txt"), b"other").unwrap();
    chmod(&unreadable, 0o000);
    chmod(&protected, 0o555);
    (tree, protected, unreadable)
}

#[test]
fn skip_keeps_the_entry_and_its_folders_with_their_own_modes_and_removes_the_rest() {
    if privileged() {
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let (tree, protected, unreadable) = tree_with_an_unreadable_folder(fixture.path());
    let (result, offered) = delete_deciding(&tree, |_| DeleteFailureChoice::Skip);
    assert_eq!(offered.len(), 1, "{offered:?}");
    assert_eq!(offered[0].error.path, unreadable);
    assert!(offered[0].can_retry);
    let done = result.unwrap();
    assert_eq!(done.skipped, 1);
    assert!(done.residue.is_none());
    // The skipped folder and the folders that contain it stay, under their own
    // names and with their original modes; everything else was removed.
    assert_eq!(mode(&unreadable), 0o000);
    assert_eq!(mode(&protected), 0o555);
    assert_eq!(mode(&tree), 0o755);
    assert!(!protected.join("gone.txt").exists());
    assert!(!tree.join("other.txt").exists());
    chmod(&protected, 0o755);
    chmod(&unreadable, 0o755);
    assert_eq!(fs::read(unreadable.join("hidden.txt")).unwrap(), b"hidden");
}

#[test]
fn retry_after_the_cause_is_fixed_where_the_item_is_shown_removes_everything() {
    if privileged() {
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let (tree, _, unreadable) = tree_with_an_unreadable_folder(fixture.path());
    let (result, offered) = delete_deciding(&tree, |failure| {
        // While the decision waits, the rest of the item is under its own name.
        assert!(tree.exists(), "the item is shown while the decision waits");
        assert_eq!(failure.error.path, unreadable);
        chmod(&unreadable, 0o755);
        DeleteFailureChoice::Retry
    });
    assert_eq!(offered.len(), 1);
    let done = result.unwrap();
    assert_eq!(done.skipped, 0);
    assert!(!tree.exists());
}

#[test]
fn cancel_keeps_what_is_left_with_the_original_modes() {
    if privileged() {
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let (tree, protected, unreadable) = tree_with_an_unreadable_folder(fixture.path());
    let (result, offered) = delete_deciding(&tree, |_| DeleteFailureChoice::Cancel);
    assert_eq!(offered.len(), 1);
    assert_eq!(result.unwrap_err().kind, ErrorKind::Interrupted);
    assert!(tree.is_dir());
    assert_eq!(mode(&protected), 0o555);
    assert_eq!(mode(&unreadable), 0o000);
    // Nothing of the item is left in staging beside it.
    let beside: Vec<_> = fs::read_dir(fixture.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(beside, [OsString::from("tree")]);
    chmod(&protected, 0o755);
    chmod(&unreadable, 0o755);
}

#[test]
fn a_folder_that_stays_and_cannot_get_its_mode_back_is_offered_by_name() {
    if privileged() {
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let (selected, protected, unreadable) = tree_with_an_unreadable_folder(fixture.path());
    // The filesystem turns read-only before the write-protected folder, which stays with
    // the skipped folder, gets its own mode back.
    tree::FAIL_RESTORE.set(Some(rustix::io::Errno::ROFS));
    let (result, offered) = delete_deciding(&selected, |_| DeleteFailureChoice::Skip);
    let injected = tree::FAIL_RESTORE.take();
    let left = mode(&protected);
    chmod(&protected, 0o755);
    chmod(&unreadable, 0o755);
    assert_eq!(injected, None, "the folder's mode was set back");
    assert_eq!(offered.len(), 2, "{offered:?}");
    assert_eq!(offered[0].error.path, unreadable);
    let restore = &offered[1];
    assert_eq!(restore.error.path, protected);
    assert!(restore.can_retry);
    assert!(
        restore.error.message.contains("0555") && restore.error.message.contains("keeps those"),
        "{restore:?}"
    );
    assert_eq!(result.unwrap().skipped, 2);
    // It keeps the owner write and search permission it got.
    assert_eq!(left, 0o755);
}

/// A write-protected selected folder with an unreadable folder, which cannot be removed,
/// and a file that can.
fn write_protected_selection(root: &Path) -> (PathBuf, PathBuf) {
    let selected = root.join("selected");
    let unreadable = selected.join("unreadable");
    fs::create_dir_all(&unreadable).unwrap();
    fs::write(selected.join("gone.txt"), b"gone").unwrap();
    chmod(&unreadable, 0o000);
    chmod(&selected, 0o555);
    (selected, unreadable)
}

/// The mode of `path`, or None where nothing is.
fn mode_if_any(path: &Path) -> Option<u32> {
    fs::symlink_metadata(path)
        .ok()
        .map(|metadata| metadata.permissions().mode() & 0o7777)
}

#[test]
fn a_write_protected_selected_folder_that_keeps_skipped_items_returns_with_its_own_mode() {
    if privileged() {
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let (selected, unreadable) = write_protected_selection(fixture.path());
    let (result, offered) = delete_deciding(&selected, |_| DeleteFailureChoice::Skip);
    // Moving the folder back from staging needs owner write: its own mode returns after.
    let left = mode_if_any(&selected);
    let beside: Vec<_> = fs::read_dir(fixture.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    if left.is_some() {
        chmod(&selected, 0o755);
        chmod(&unreadable, 0o755);
    }
    assert_eq!(offered.len(), 1, "{offered:?}");
    assert_eq!(result.unwrap().skipped, 1);
    assert_eq!(left, Some(0o555));
    assert_eq!(beside, [OsString::from("selected")]);
    assert!(!selected.join("gone.txt").exists());
}

#[test]
fn a_selected_folder_that_cannot_get_its_mode_back_is_offered_by_name() {
    if privileged() {
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let (selected, unreadable) = write_protected_selection(fixture.path());
    tree::FAIL_RESTORE.set(Some(rustix::io::Errno::IO));
    let (result, offered) = delete_deciding(&selected, |_| DeleteFailureChoice::Skip);
    let injected = tree::FAIL_RESTORE.take();
    let left = mode_if_any(&selected);
    if left.is_some() {
        chmod(&selected, 0o755);
        chmod(&unreadable, 0o755);
    }
    assert_eq!(injected, None, "the folder's mode was set back");
    assert_eq!(offered.len(), 2, "{offered:?}");
    assert_eq!(offered[1].error.path, selected);
    assert!(
        offered[1].error.message.contains("0555"),
        "{:?}",
        offered[1]
    );
    // The folder's mode is not an entry left behind.
    assert_eq!(result.unwrap().skipped, 1);
    assert_eq!(left, Some(0o755));
}

#[test]
fn a_set_group_id_folder_that_stays_keeps_the_bit() {
    if privileged() {
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let (selected, protected, unreadable) = tree_with_an_unreadable_folder(fixture.path());
    // In the caller's own group the bit survives owner write and the mode set back.
    chmod(&protected, 0o2555);
    if mode(&protected) != 0o2555 {
        chmod(&protected, 0o755);
        chmod(&unreadable, 0o755);
        eprintln!("SKIP: the folder's group is not the caller's, or the filesystem drops the bit");
        return;
    }
    let (result, offered) = delete_deciding(&selected, |_| DeleteFailureChoice::Skip);
    let left = mode(&protected);
    chmod(&protected, 0o755);
    chmod(&unreadable, 0o755);
    assert_eq!(offered.len(), 1, "{offered:?}");
    assert_eq!(result.unwrap().skipped, 1);
    assert_eq!(left, 0o2555);
    // Outside the folder's group a change would clear it: such a folder gets no owner
    // write (`crate::ownership::keeps_set_group_id`), and the walk says why.
    assert!(!crate::ownership::keeps_set_group_id(0o2555, 4242, |_| {
        false
    }));
    assert!(tree::SET_GROUP_ID_KEPT.contains("set-group-ID"));
}
