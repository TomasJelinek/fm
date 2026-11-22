//! A file or folder inside a copied folder that cannot be opened pauses alone:
//! Skip leaves out only that child, Retry opens it again, and the rest of its
//! folder is still copied. A selected item that cannot be prepared is skipped the
//! same way, and the other selected items are copied; so is an item that a Move cannot
//! take out of its write-protected folder (T-E06).
use super::*;
use std::os::unix::fs::PermissionsExt;

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// Whether permissions bind this process; they do not bind the superuser.
fn privileged(folder: &Path) -> bool {
    let probe = folder.join("probe");
    fs::write(&probe, b"").unwrap();
    chmod(&probe, 0o000);
    let readable = fs::File::open(&probe).is_ok();
    fs::remove_file(&probe).unwrap();
    if readable {
        eprintln!("SKIP: permissions do not apply to the superuser");
    }
    readable
}

/// `tree` holds readable files and a readable folder around the unreadable
/// children `names`; a name ending in `/` is a folder with a file inside.
fn tree(root: &Path, names: &[&str]) -> PathBuf {
    let tree = root.join("tree");
    fs::create_dir_all(tree.join("open")).unwrap();
    fs::write(tree.join("open/inside.txt"), b"inside").unwrap();
    for name in ["a.txt", "b.txt"] {
        fs::write(tree.join(name), name).unwrap();
    }
    for name in names {
        let path = tree.join(name.trim_end_matches('/'));
        if name.ends_with('/') {
            fs::create_dir(&path).unwrap();
            fs::write(path.join("hidden.txt"), b"hidden").unwrap();
        } else {
            fs::write(&path, name).unwrap();
        }
        chmod(&path, 0o000);
    }
    tree
}

/// Make every child readable again, as the user fixing it would.
fn unlock(tree: &Path, names: &[&str]) {
    for name in names {
        let path = tree.join(name.trim_end_matches('/'));
        chmod(&path, if name.ends_with('/') { 0o755 } else { 0o644 });
    }
}

async fn failure(h: &mut Harness, id: u64) -> Arc<CopyFailure> {
    loop {
        match h.next().await {
            CopyEvent::Failure(failure) if failure.id == OperationId(id) => return failure,
            CopyEvent::Progress(progress) if progress.id == OperationId(id) => {
                assert!(
                    !progress.state.is_terminal(),
                    "expected a failure: {progress:?}"
                );
            }
            _ => {}
        }
    }
}

/// The job's end, failing on any further question.
async fn finished(h: &mut Harness, id: u64) -> Arc<CopyProgress> {
    loop {
        match h.next().await {
            CopyEvent::Failure(failure) => panic!("asked again: {failure:?}"),
            CopyEvent::Progress(progress)
                if progress.id == OperationId(id) && progress.state.is_terminal() =>
            {
                return progress;
            }
            _ => {}
        }
    }
}

async fn decide(h: &Harness, failure: &CopyFailure, choice: CopyChoice) {
    h.controller
        .decide(CopyDecision {
            id: failure.id,
            version: failure.version,
            choice,
        })
        .await;
}

fn assert_rest_copied(copied: &Path) {
    assert_eq!(fs::read(copied.join("a.txt")).unwrap(), b"a.txt");
    assert_eq!(fs::read(copied.join("b.txt")).unwrap(), b"b.txt");
    assert_eq!(fs::read(copied.join("open/inside.txt")).unwrap(), b"inside");
}

#[tokio::test]
async fn skip_leaves_out_only_the_child_that_cannot_be_read_and_copies_the_rest() {
    let temp = fixture();
    if privileged(temp.path()) {
        return;
    }
    let names = ["locked/", "unreadable.txt"];
    let tree = tree(temp.path(), &names);
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&tree), &destination))
        .await;
    let mut skipped = Vec::new();
    for _ in names {
        let failure = failure(&mut h, 1).await;
        assert_eq!(failure.stage, CopyErrorStage::Reading, "{failure:?}");
        assert!(failure.can_retry && failure.can_skip, "{failure:?}");
        assert_eq!(failure.error.kind, io::ErrorKind::PermissionDenied);
        let source = failure.source.clone().expect("the child is named");
        assert_eq!(
            failure.destination,
            destination.join("tree").join(source.file_name().unwrap())
        );
        skipped.push(source);
        decide(&h, &failure, CopyChoice::Skip).await;
    }
    let done = finished(&mut h, 1).await;
    unlock(&tree, &names);
    skipped.sort();
    assert_eq!(skipped, [tree.join("locked"), tree.join("unreadable.txt")]);
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!((done.skipped, done.failed), (2, 2), "{done:?}");
    let copied = destination.join("tree");
    assert_rest_copied(&copied);
    assert!(!copied.join("locked").exists());
    assert!(!copied.join("unreadable.txt").exists());
    h.close().await;
}

#[tokio::test]
async fn retry_copies_the_child_once_it_can_be_read() {
    let temp = fixture();
    if privileged(temp.path()) {
        return;
    }
    let names = ["locked/", "unreadable.txt"];
    let tree = tree(temp.path(), &names);
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&tree), &destination))
        .await;
    for _ in names {
        let failure = failure(&mut h, 1).await;
        assert!(failure.can_retry, "{failure:?}");
        let source = failure.source.clone().expect("the child is named");
        let name = source.file_name().unwrap().to_str().unwrap();
        let name = if name == "locked" { "locked/" } else { name };
        unlock(&tree, &[name]);
        decide(&h, &failure, CopyChoice::Retry).await;
    }
    let done = finished(&mut h, 1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!((done.skipped, done.failed), (0, 0), "{done:?}");
    let copied = destination.join("tree");
    assert_rest_copied(&copied);
    assert_eq!(
        fs::read(copied.join("locked/hidden.txt")).unwrap(),
        b"hidden"
    );
    assert_eq!(
        fs::read(copied.join("unreadable.txt")).unwrap(),
        b"unreadable.txt"
    );
    h.close().await;
}

#[tokio::test]
async fn skip_all_errors_leaves_out_later_unreadable_children_without_asking() {
    let temp = fixture();
    if privileged(temp.path()) {
        return;
    }
    let names = ["first.txt", "second.txt", "third.txt"];
    let tree = tree(temp.path(), &names);
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&tree), &destination))
        .await;
    let first = failure(&mut h, 1).await;
    decide(&h, &first, CopyChoice::SkipAllErrors).await;
    let done = finished(&mut h, 1).await;
    unlock(&tree, &names);
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!((done.skipped, done.failed), (3, 3), "{done:?}");
    let copied = destination.join("tree");
    assert_rest_copied(&copied);
    for name in names {
        assert!(!copied.join(name).exists(), "{name}");
    }
    h.close().await;
}

#[tokio::test]
async fn a_selected_item_that_cannot_be_prepared_is_skipped_and_the_others_are_copied() {
    let temp = fixture();
    if privileged(temp.path()) {
        return;
    }
    let locked = temp.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("hidden.txt"), b"hidden").unwrap();
    let file = temp.path().join("file.txt");
    fs::write(&file, b"file").unwrap();
    chmod(&locked, 0o000);
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, &[locked.clone(), file], &destination))
        .await;
    let failure = failure(&mut h, 1).await;
    let skippable = (failure.stage, failure.can_retry, failure.can_skip);
    let source = failure.source.clone();
    decide(&h, &failure, CopyChoice::Skip).await;
    let done = finished(&mut h, 1).await;
    chmod(&locked, 0o755);
    assert_eq!(
        skippable,
        (CopyErrorStage::Preparing, true, true),
        "{failure:?}"
    );
    assert_eq!(source, Some(locked));
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!((done.skipped, done.failed), (1, 1), "{done:?}");
    assert_eq!(fs::read(destination.join("file.txt")).unwrap(), b"file");
    assert!(!destination.join("locked").exists());
    h.close().await;
}

#[tokio::test]
async fn a_single_selected_item_that_cannot_be_prepared_can_be_skipped_like_any_other() {
    // T-E06: Skip leaves the one selected item where it is; the job then has nothing
    // left to do and ends at once, with the skip among its errors.
    let temp = fixture();
    if privileged(temp.path()) {
        return;
    }
    let locked = temp.path().join("locked");
    fs::create_dir(&locked).unwrap();
    chmod(&locked, 0o000);
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&locked), &destination))
        .await;
    let failure = failure(&mut h, 1).await;
    decide(&h, &failure, CopyChoice::Skip).await;
    let done = h.terminal(1).await;
    chmod(&locked, 0o755);
    assert!(failure.can_retry && failure.can_skip, "{failure:?}");
    assert_eq!(failure.stage, CopyErrorStage::Preparing, "{failure:?}");
    assert_eq!(done.state, CopyOperationState::Failed, "{done:?}");
    assert_eq!((done.skipped, done.failed), (1, 1), "{done:?}");
    assert!(locked.is_dir());
    assert!(!destination.join("locked").exists());
    h.close().await;
}

/// A Move of `held.txt` out of the write-protected (mode 0555) folder `ro-src`.
async fn protected_move(temp: &Path) -> (Harness, PathBuf, PathBuf) {
    let folder = temp.join("ro-src");
    fs::create_dir(&folder).unwrap();
    let held = folder.join("held.txt");
    fs::write(&held, b"held").unwrap();
    let destination = temp.join("destination");
    fs::create_dir(&destination).unwrap();
    chmod(&folder, 0o555);
    let h = Harness::native();
    h.controller
        .enqueue_move(request(1, std::slice::from_ref(&held), &destination))
        .await;
    (h, held, destination)
}

#[tokio::test]
async fn a_move_out_of_a_write_protected_folder_offers_skip_and_skip_leaves_the_item() {
    // T-E06: a question, not a failed job, with Retry, Skip and Skip all this error type.
    let temp = fixture();
    if privileged(temp.path()) {
        return;
    }
    let (mut h, held, destination) = protected_move(temp.path()).await;
    let failure = failure(&mut h, 1).await;
    decide(&h, &failure, CopyChoice::Skip).await;
    let done = h.terminal(1).await;
    chmod(held.parent().unwrap(), 0o755);
    assert!(failure.can_retry && failure.can_skip, "{failure:?}");
    assert_eq!(failure.stage, CopyErrorStage::Preparing, "{failure:?}");
    assert_eq!(failure.source.as_deref(), Some(held.as_path()));
    assert_eq!(failure.error.kind, io::ErrorKind::PermissionDenied);
    assert!(
        failure
            .error
            .message
            .starts_with("Permission denied: it cannot be moved out of its folder"),
        "{failure:?}"
    );
    assert!(!failure.error.message.contains("os error"), "{failure:?}");
    assert_eq!(done.state, CopyOperationState::Failed, "{done:?}");
    assert_eq!((done.skipped, done.failed), (1, 1), "{done:?}");
    assert_eq!(fs::read(&held).unwrap(), b"held");
    assert!(!destination.join("held.txt").exists());
    h.close().await;
}

#[tokio::test]
async fn retry_moves_the_item_once_its_folder_is_writable() {
    let temp = fixture();
    if privileged(temp.path()) {
        return;
    }
    let (mut h, held, destination) = protected_move(temp.path()).await;
    let failure = failure(&mut h, 1).await;
    chmod(held.parent().unwrap(), 0o755);
    decide(&h, &failure, CopyChoice::Retry).await;
    let done = finished(&mut h, 1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert!(!held.exists());
    assert_eq!(fs::read(destination.join("held.txt")).unwrap(), b"held");
    h.close().await;
}
