//! Delete failures wait for a decision (Retry, Skip, Skip all errors, Cancel), and the
//! job then goes on with every remaining entry and target.
use super::*;

/// A permanent Delete of `names` in the fixture folder, selected together.
fn deleting(names: &[&str]) -> Harness {
    let path = std::env::temp_dir().join("midnight-delete-decision-fixture");
    let entries: Arc<[Entry]> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            Entry::new(EntryId(index as u64 + 1), path.join(name), EntryKind::File)
        })
        .collect();
    let mut h = Harness::in_directory(
        MutationKind::Delete(DeleteMode::Permanent),
        None,
        path,
        entries.clone(),
    );
    h.request.selection = Arc::new(
        SelectionSnapshot::from_bits(h.request.view.clone(), vec![(1u64 << entries.len()) - 1])
            .unwrap(),
    );
    h.request.focused = None;
    h
}

impl Harness {
    fn target(&self, index: usize) -> PathBuf {
        self.request.source.entries[index].path.clone()
    }
    async fn start(&mut self) {
        self.controller.prepare(self.request.clone()).await;
        self.preview().await;
        self.controller.confirm(self.request.id, None).await;
    }
    /// The next failure, which the job must ask about before it finishes.
    async fn failure(&mut self) -> Arc<DeleteFailure> {
        loop {
            match self.next().await {
                OperationEvent::DeleteFailure(failure) => return failure,
                OperationEvent::Progress(progress) => {
                    assert!(
                        !progress.state.is_terminal(),
                        "finished without asking: {progress:?}"
                    );
                }
                _ => {}
            }
        }
    }
    async fn decide(&self, failure: &DeleteFailure, choice: CopyChoice) {
        self.controller
            .decide_delete(CopyDecision {
                id: failure.id,
                version: failure.version,
                choice,
            })
            .await;
    }
    fn deleted(&self) -> Vec<PathBuf> {
        let deleted = self.calls.deleted.lock().unwrap();
        deleted.iter().map(|(path, _)| path.clone()).collect()
    }
}

/// The preview and the progress that preparation published before it.
async fn prepared(h: &mut Harness) -> (Arc<OperationPreview>, Arc<OperationProgress>) {
    h.controller.prepare(h.request.clone()).await;
    let mut awaiting = None;
    loop {
        match h.next().await {
            OperationEvent::Preview(preview) => {
                return (preview, awaiting.expect("progress before the preview"));
            }
            OperationEvent::Progress(progress) => awaiting = Some(progress),
            _ => {}
        }
    }
}

/// F8 of items whose folder no Trash takes (the drive's Recycle Bin is turned off): the
/// job is prepared as a permanent delete, whose confirmation says why, before anything
/// starts. Preparation asks once for the folder; once confirmed, every item is deleted
/// permanently and nothing more is asked.
#[tokio::test]
async fn a_trash_delete_that_no_trash_takes_is_prepared_as_a_permanent_delete() {
    for (names, sentence) in [
        (
            &["only"][..],
            "The Recycle Bin is turned off for X:, so this item will be deleted permanently.",
        ),
        (
            &["first", "second", "third"][..],
            "The Recycle Bin is turned off for X:, so these items will be deleted permanently.",
        ),
    ] {
        let mut h = deleting(names);
        h.request.kind = MutationKind::Delete(DeleteMode::Trash);
        let folder = h.request.source.path.clone();
        h.calls.no_trash.lock().unwrap().push(folder);
        let (preview, awaiting) = prepared(&mut h).await;
        let permanent = MutationKind::Delete(DeleteMode::Permanent);
        assert_eq!(preview.kind, permanent);
        assert_eq!(preview.trash_unavailable.as_deref(), Some(sentence));
        assert_eq!(preview.files, names.len() as u64);
        assert_eq!(
            (awaiting.kind, awaiting.state),
            (permanent, OperationState::AwaitingConfirmation)
        );
        assert_eq!(h.calls.trash_asked.load(Ordering::Relaxed), 1, "one folder");
        assert!(h.deleted().is_empty(), "nothing before the confirmation");
        h.controller.confirm(h.request.id, None).await;
        let (done, asked) = h.finished_deciding(CopyChoice::Cancel).await;
        assert!(asked.is_empty(), "{asked:?}");
        assert_eq!(
            (done.state, done.kind),
            (OperationState::Completed, permanent)
        );
        let deleted = h.calls.deleted.lock().unwrap().clone();
        assert_eq!(deleted.len(), names.len());
        assert!(
            deleted
                .iter()
                .all(|(_, mode)| *mode == DeleteMode::Permanent),
            "{deleted:?}"
        );
    }
}

/// A Trash delete whose folder has a Trash stays one; a permanent delete never asks.
#[tokio::test]
async fn a_trash_delete_with_a_trash_stays_one_and_a_permanent_one_never_asks() {
    let mut h = deleting(&["a", "b"]);
    h.request.kind = MutationKind::Delete(DeleteMode::Trash);
    let (preview, awaiting) = prepared(&mut h).await;
    let trash = MutationKind::Delete(DeleteMode::Trash);
    assert_eq!((preview.kind, awaiting.kind), (trash, trash));
    assert_eq!(preview.trash_unavailable, None);
    assert_eq!(h.calls.trash_asked.load(Ordering::Relaxed), 1);
    h.controller.confirm(h.request.id, None).await;
    let (done, asked) = h.finished_deciding(CopyChoice::Cancel).await;
    assert!(asked.is_empty(), "{asked:?}");
    assert_eq!(done.state, OperationState::Completed);
    assert!(
        h.calls
            .deleted
            .lock()
            .unwrap()
            .iter()
            .all(|(_, mode)| *mode == DeleteMode::Trash)
    );

    let mut h = deleting(&["a"]);
    let folder = h.request.source.path.clone();
    h.calls.no_trash.lock().unwrap().push(folder);
    let (preview, _) = prepared(&mut h).await;
    assert_eq!(preview.kind, MutationKind::Delete(DeleteMode::Permanent));
    assert_eq!(
        preview.trash_unavailable, None,
        "Shift+Delete says nothing more"
    );
    assert_eq!(h.calls.trash_asked.load(Ordering::Relaxed), 0);
}

/// A Trash that refuses an item while the job runs (its setting changed after
/// preparation, or the item is too large for it) is an ordinary failure: the item is
/// kept, Retry, Skip and Cancel decide, Retry works once the Trash takes it, and nothing
/// is ever deleted permanently instead.
#[tokio::test]
async fn a_trash_refusal_during_the_job_is_an_ordinary_failure_that_keeps_the_item() {
    let mut h = deleting(&["refused", "recycled"]);
    h.request.kind = MutationKind::Delete(DeleteMode::Trash);
    let refused = h.target(0);
    *h.calls.trash_unavailable.lock().unwrap() = vec![refused.clone()];
    h.start().await;
    let failure = h.failure().await;
    assert_eq!(failure.error.path, refused);
    assert!(failure.error.is_trash_unavailable(), "{failure:?}");
    assert_eq!(failure.mode, DeleteMode::Trash);
    assert!(failure.can_retry && failure.can_skip);
    h.calls.trash_unavailable.lock().unwrap().clear();
    h.decide(&failure, CopyChoice::Retry).await;
    let (done, asked) = h.finished_deciding(CopyChoice::Cancel).await;
    assert!(asked.is_empty(), "{asked:?}");
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!((done.completed, done.failed), (2, 0));
    assert_eq!(
        *h.calls.deleted.lock().unwrap(),
        [
            (refused.clone(), DeleteMode::Trash),
            (refused, DeleteMode::Trash),
            (h.target(1), DeleteMode::Trash),
        ]
    );

    // The only item: still asked (Retry may help), and Skip keeps it.
    let mut h = deleting(&["refused"]);
    h.request.kind = MutationKind::Delete(DeleteMode::Trash);
    let refused = h.target(0);
    *h.calls.trash_unavailable.lock().unwrap() = vec![refused.clone()];
    h.start().await;
    let failure = h.failure().await;
    assert!(failure.can_retry && failure.can_skip);
    h.decide(&failure, CopyChoice::Skip).await;
    let done = h.finished().await;
    assert_eq!(done.state, OperationState::CompletedWithErrors);
    assert_eq!((done.completed, done.failed), (0, 1));
    assert!(done.errors[0].is_trash_unavailable());
    assert_eq!(
        *h.calls.deleted.lock().unwrap(),
        [(refused, DeleteMode::Trash)],
        "never a permanent delete"
    );
}

#[tokio::test]
async fn retry_removes_an_entry_once_its_folder_is_writable_and_counts_no_error() {
    let mut h = deleting(&["tree", "other"]);
    let tree = h.target(0);
    h.calls
        .entry_failures
        .lock()
        .unwrap()
        .insert(tree.clone(), 1);
    h.start().await;
    let failure = h.failure().await;
    assert_eq!(failure.error.path, tree.join("locked-0"));
    assert_eq!(failure.mode, DeleteMode::Permanent);
    assert!(failure.can_retry && failure.can_skip);
    // Retrying before anything changed asks again, with a newer version.
    h.decide(&failure, CopyChoice::Retry).await;
    let again = h.failure().await;
    assert!(again.version > failure.version);
    h.calls.writable.store(true, Ordering::Release);
    h.decide(&again, CopyChoice::Retry).await;
    let (done, asked) = h.finished_deciding(CopyChoice::Cancel).await;
    assert!(asked.is_empty(), "{asked:?}");
    assert_eq!(done.state, OperationState::Completed, "{done:?}");
    assert_eq!((done.total, done.completed, done.failed), (2, 2, 0));
    assert!(done.errors.is_empty());
    assert_eq!(h.deleted(), [tree, h.target(1)]);
}

#[tokio::test]
async fn skip_keeps_that_entry_and_its_target_and_deletes_every_other_target() {
    let mut h = deleting(&["tree", "other"]);
    let tree = h.target(0);
    h.calls
        .entry_failures
        .lock()
        .unwrap()
        .insert(tree.clone(), 1);
    h.start().await;
    let failure = h.failure().await;
    h.decide(&failure, CopyChoice::Skip).await;
    let (done, asked) = h.finished_deciding(CopyChoice::Cancel).await;
    assert!(asked.is_empty(), "{asked:?}");
    assert_eq!(done.state, OperationState::CompletedWithErrors);
    // One error, the skipped entry; its target is not deleted, and says so once.
    assert_eq!((done.total, done.completed, done.failed), (2, 1, 1));
    assert_eq!(done.errors.len(), 1);
    assert_eq!(done.errors[0].path, tree.join("locked-0"));
    let kept = format!(
        "{} was not deleted because it still contains skipped items.",
        tree.display()
    );
    assert_eq!(done.message.matches(&kept).count(), 1, "{}", done.message);
    assert_eq!(h.deleted(), [h.target(1)]);
}

#[tokio::test]
async fn skip_all_errors_skips_the_later_failures_of_the_job_without_asking() {
    let mut h = deleting(&["tree", "other", "last"]);
    let (tree, other, last) = (h.target(0), h.target(1), h.target(2));
    {
        let mut failures = h.calls.entry_failures.lock().unwrap();
        failures.insert(tree.clone(), 2);
        failures.insert(other.clone(), 1);
    }
    // The last target fails as a whole.
    *h.calls.failing_delete.lock().unwrap() = Some(last.clone());
    h.start().await;
    let failure = h.failure().await;
    assert_eq!(failure.error.path, tree.join("locked-0"));
    h.decide(&failure, CopyChoice::SkipAllErrors).await;
    let (done, asked) = h.finished_deciding(CopyChoice::Cancel).await;
    assert!(asked.is_empty(), "nothing more is asked: {asked:?}");
    assert_eq!(h.calls.offered.load(Ordering::Acquire), 3);
    assert_eq!(done.state, OperationState::CompletedWithErrors);
    // Three skipped entries and the failed target, each one error.
    assert_eq!((done.total, done.completed, done.failed), (3, 0, 4));
    let paths: Vec<_> = done.errors.iter().map(|error| error.path.clone()).collect();
    assert_eq!(
        paths,
        [
            tree.join("locked-0"),
            tree.join("locked-1"),
            other.join("locked-0"),
            last.clone()
        ]
    );
    for target in [&tree, &other] {
        assert!(done.message.contains(&format!(
            "{} was not deleted because it still contains skipped items.",
            target.display()
        )));
    }
    assert!(
        !done
            .message
            .contains(&format!("{} was not deleted because", last.display()))
    );
    assert_eq!(h.deleted(), [last]);
}

#[tokio::test]
async fn cancel_ends_the_job_and_keeps_every_target_it_did_not_reach() {
    let mut h = deleting(&["tree", "other"]);
    let tree = h.target(0);
    h.calls
        .entry_failures
        .lock()
        .unwrap()
        .insert(tree.clone(), 1);
    h.start().await;
    let failure = h.failure().await;
    h.decide(&failure, CopyChoice::Cancel).await;
    let done = h.finished().await;
    assert_eq!(done.state, OperationState::Cancelled);
    assert_eq!((done.completed, done.failed), (0, 1));
    assert_eq!(done.errors[0].path, tree.join("locked-0"));
    assert!(
        h.deleted().is_empty(),
        "nothing after the cancelled failure"
    );
}

#[tokio::test]
async fn a_target_that_failed_as_a_whole_can_be_retried() {
    let mut h = deleting(&["first", "second"]);
    let first = h.target(0);
    *h.calls.failing_delete.lock().unwrap() = Some(first.clone());
    h.start().await;
    let failure = h.failure().await;
    assert_eq!(failure.error.path, first);
    assert!(failure.can_retry);
    *h.calls.failing_delete.lock().unwrap() = None;
    h.decide(&failure, CopyChoice::Retry).await;
    let (done, asked) = h.finished_deciding(CopyChoice::Cancel).await;
    assert!(asked.is_empty(), "{asked:?}");
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!((done.completed, done.failed), (2, 0));
    assert_eq!(h.deleted(), [first.clone(), first, h.target(1)]);
}

/// The real Windows provider: a file that another program keeps open decides.
#[cfg(windows)]
#[path = "delete_decision_windows_tests.rs"]
mod native_windows;

/// The real Linux provider: permissions decide, so these need an ordinary user.
#[cfg(target_os = "linux")]
mod native {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    /// Whether folder permissions bind this process (they do not bind the superuser).
    fn privileged() -> bool {
        let probe = tempfile::tempdir().unwrap();
        chmod(probe.path(), 0o555);
        let written = fs::write(probe.path().join("probe"), b"").is_ok();
        chmod(probe.path(), 0o755);
        written
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    fn chmod(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[tokio::test]
    async fn a_write_protected_folder_inside_the_tree_is_deleted_without_asking() {
        if privileged() {
            eprintln!("SKIP: permissions do not apply to the superuser");
            return;
        }
        let fixture = tempfile::tempdir().unwrap();
        let tree = fixture.path().join("tree");
        let locked = tree.join("readonly").join("locked-folder");
        fs::create_dir_all(&locked).unwrap();
        fs::write(locked.join("file.txt"), b"inside").unwrap();
        fs::write(tree.join("readonly").join("note.txt"), b"beside").unwrap();
        chmod(&locked, 0o555);
        chmod(&tree.join("readonly"), 0o555);
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
        assert_eq!(done.processed_entries, 5);
        assert!(!tree.exists());
    }

    #[tokio::test]
    async fn a_write_protected_containing_folder_asks_and_is_never_changed() {
        if privileged() {
            eprintln!("SKIP: permissions do not apply to the superuser");
            return;
        }
        let fixture = tempfile::tempdir().unwrap();
        let parent = fixture.path().join("protected");
        let item = parent.join("item");
        fs::create_dir_all(&item).unwrap();
        fs::write(item.join("file.txt"), b"kept").unwrap();
        chmod(&parent, 0o555);
        let mut h = Harness::native(
            MutationKind::Delete(DeleteMode::Permanent),
            &parent,
            Arc::from([Entry::new(EntryId(7), item.clone(), EntryKind::Directory)]),
        );
        h.start().await;
        let failure = h.failure().await;
        assert_eq!(failure.error.path, parent);
        assert!(
            failure.error.message.contains("write-protected"),
            "{failure:?}"
        );
        h.decide(&failure, CopyChoice::Skip).await;
        let done = h.finished().await;
        assert_eq!(done.state, OperationState::CompletedWithErrors);
        assert_eq!((done.completed, done.failed), (0, 1));
        assert_eq!(
            mode(&parent),
            0o555,
            "the containing folder is never changed"
        );
        assert_eq!(fs::read(item.join("file.txt")).unwrap(), b"kept");
        chmod(&parent, 0o755);
    }
}
