//! The real Windows provider: a file that another program keeps open without delete
//! sharing pauses the Delete for that file alone, and Skip keeps it with the folders
//! that contain it while everything else is deleted, the other targets too. Read-only
//! files and folders of the confirmed tree never ask.
use super::*;
use std::{fs, os::windows::fs::OpenOptionsExt};

/// Opens `path` as a program that does not let it be deleted while it is open.
fn lock(path: &Path) -> fs::File {
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    fs::OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ_WRITE)
        .open(path)
        .unwrap()
}

fn set_read_only(path: &Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).unwrap();
}

/// A permanent Delete of `targets` in `folder`, selected together.
fn deleting(folder: &Path, targets: &[(&Path, EntryKind)]) -> Harness {
    let entries: Arc<[Entry]> = targets
        .iter()
        .enumerate()
        .map(|(index, (path, kind))| {
            Entry::new(EntryId(index as u64 + 1), path.to_path_buf(), *kind)
        })
        .collect();
    let mut h = Harness::native(
        MutationKind::Delete(DeleteMode::Permanent),
        folder,
        entries.clone(),
    );
    h.request.selection = Arc::new(
        SelectionSnapshot::from_bits(h.request.view.clone(), vec![(1u64 << entries.len()) - 1])
            .unwrap(),
    );
    h.request.focused = None;
    h
}

/// `tree` whose folder `nested` holds `locked.txt` beside entries that can be removed.
fn tree_with_a_locked_file(root: &Path) -> (PathBuf, PathBuf) {
    let tree = root.join("tree");
    let nested = tree.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("locked.txt"), b"locked").unwrap();
    fs::write(nested.join("gone.txt"), b"gone").unwrap();
    fs::write(tree.join("other.txt"), b"other").unwrap();
    (tree, nested.join("locked.txt"))
}

#[tokio::test]
async fn skip_keeps_a_locked_file_with_its_folders_and_deletes_everything_else() {
    let fixture = tempfile::tempdir().unwrap();
    let (tree, locked) = tree_with_a_locked_file(fixture.path());
    let last = fixture.path().join("last.txt");
    fs::write(&last, b"last").unwrap();
    let held = lock(&locked);
    let mut h = deleting(
        fixture.path(),
        &[(&tree, EntryKind::Directory), (&last, EntryKind::File)],
    );
    h.start().await;
    let failure = h.failure().await;
    // The file itself is asked about, not the selected folder.
    assert_eq!(failure.error.path, locked);
    assert_eq!(failure.mode, DeleteMode::Permanent);
    assert!(failure.can_retry && failure.can_skip, "{failure:?}");
    assert!(
        failure.error.message.contains("(os error 32)"),
        "{failure:?}"
    );
    h.decide(&failure, CopyChoice::Skip).await;
    let done = h.finished().await;
    drop(held);
    assert_eq!(done.state, OperationState::CompletedWithErrors, "{done:?}");
    // One error, the skipped file; its target is not deleted, and says so once.
    assert_eq!((done.total, done.completed, done.failed), (2, 1, 1));
    assert_eq!(done.errors.len(), 1);
    assert_eq!(done.errors[0].path, locked);
    let kept = format!(
        "{} was not deleted because it still contains skipped items.",
        tree.display()
    );
    assert_eq!(done.message.matches(&kept).count(), 1, "{}", done.message);
    assert_eq!(fs::read(&locked).unwrap(), b"locked");
    assert!(!locked.with_file_name("gone.txt").exists());
    assert!(!tree.join("other.txt").exists());
    assert!(!last.exists());
}

#[tokio::test]
async fn retry_deletes_the_file_once_it_is_closed_and_counts_no_error() {
    let fixture = tempfile::tempdir().unwrap();
    let (tree, locked) = tree_with_a_locked_file(fixture.path());
    let held = lock(&locked);
    let mut h = Harness::native(
        MutationKind::Delete(DeleteMode::Permanent),
        fixture.path(),
        Arc::from([Entry::new(EntryId(7), tree.clone(), EntryKind::Directory)]),
    );
    h.start().await;
    let failure = h.failure().await;
    assert_eq!(failure.error.path, locked);
    // Retrying while the file is still open asks again, with a newer version.
    h.decide(&failure, CopyChoice::Retry).await;
    let again = h.failure().await;
    assert!(again.version > failure.version, "{again:?}");
    assert_eq!(again.error.path, locked);
    drop(held);
    h.decide(&again, CopyChoice::Retry).await;
    let (done, asked) = h.finished_deciding(CopyChoice::Cancel).await;
    assert!(asked.is_empty(), "{asked:?}");
    assert_eq!(done.state, OperationState::Completed, "{done:?}");
    assert_eq!((done.completed, done.failed), (1, 0));
    assert!(done.errors.is_empty(), "{done:?}");
    assert!(!tree.exists());
}

#[tokio::test]
async fn read_only_files_and_folders_inside_the_tree_are_deleted_without_asking() {
    let fixture = tempfile::tempdir().unwrap();
    let tree = fixture.path().join("tree");
    let folder = tree.join("read-only folder");
    fs::create_dir_all(&folder).unwrap();
    for file in [tree.join("note.txt"), folder.join("file.txt")] {
        fs::write(&file, b"read-only").unwrap();
        set_read_only(&file);
    }
    set_read_only(&folder);
    set_read_only(&tree);
    let mut h = Harness::native(
        MutationKind::Delete(DeleteMode::Permanent),
        fixture.path(),
        Arc::from([Entry::new(EntryId(7), tree.clone(), EntryKind::Directory)]),
    );
    h.start().await;
    let (done, asked) = h.finished_deciding(CopyChoice::Cancel).await;
    assert!(asked.is_empty(), "{asked:?}");
    assert_eq!(done.state, OperationState::Completed, "{done:?}");
    assert_eq!((done.completed, done.failed), (1, 0));
    assert_eq!(done.processed_entries, 4);
    assert!(!tree.exists());
}
