//! Windows: a file or folder inside a copied folder that cannot be opened pauses alone
//! (stage Reading): Skip leaves out only that child, Retry opens it again, and the rest
//! of its folder is still copied. An access-denied child and a file that another
//! program writes are the typical cases.
use super::*;
use std::os::windows::fs::OpenOptionsExt;

/// Runs `icacls` from System32 without a shell; the path stays a literal argument.
fn icacls(path: &Path, arguments: &[&str]) -> std::process::Output {
    let system =
        PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot")).join("System32");
    std::process::Command::new(system.join("icacls.exe"))
        .arg(path)
        .args(arguments)
        .output()
        .unwrap()
}

/// Denies reading (a file's content, a folder's listing) to Everyone until dropped.
struct Unreadable(PathBuf);

impl Unreadable {
    fn new(path: &Path) -> Self {
        let output = icacls(path, &["/deny", "*S-1-1-0:(RD)"]);
        assert!(output.status.success(), "fixture ACL: {output:?}");
        Self(path.to_owned())
    }
}

impl Drop for Unreadable {
    fn drop(&mut self) {
        // Removes the deny entries for Everyone again, so the fixture can be removed.
        let output = icacls(&self.0, &["/remove:d", "*S-1-1-0"]);
        if !output.status.success() {
            eprintln!("cannot restore fixture ACL: {output:?}");
        }
    }
}

/// Holds `path` open for writing, as another program writing the file would: Copy
/// reads a file only while nobody writes it.
fn written(path: &Path) -> fs::File {
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    fs::OpenOptions::new()
        .write(true)
        .share_mode(SHARE_READ_WRITE)
        .open(path)
        .unwrap()
}

/// `tree` holds readable files and a readable folder around the children `names`; a
/// name ending in `/` is a folder with a file inside.
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
    }
    tree
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
async fn skip_leaves_out_only_the_children_that_cannot_be_opened_and_copies_the_rest() {
    let temp = fixture();
    let names = ["locked/", "unreadable.txt", "written.txt"];
    let tree = tree(temp.path(), &names);
    let denied = [
        Unreadable::new(&tree.join("locked")),
        Unreadable::new(&tree.join("unreadable.txt")),
    ];
    let writer = written(&tree.join("written.txt"));
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
        let source = failure.source.clone().expect("the child is named");
        assert_eq!(source.parent(), Some(tree.as_path()), "{failure:?}");
        assert_eq!(
            failure.destination,
            destination.join("tree").join(source.file_name().unwrap())
        );
        if source.ends_with("written.txt") {
            assert!(
                failure.error.message.contains("(os error 32)"),
                "{failure:?}"
            );
        } else {
            assert_eq!(failure.error.kind, io::ErrorKind::PermissionDenied);
        }
        skipped.push(source);
        decide(&h, &failure, CopyChoice::Skip).await;
    }
    let done = finished(&mut h, 1).await;
    drop((denied, writer));
    skipped.sort();
    assert_eq!(
        skipped,
        [
            tree.join("locked"),
            tree.join("unreadable.txt"),
            tree.join("written.txt")
        ]
    );
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!((done.skipped, done.failed), (3, 3), "{done:?}");
    let copied = destination.join("tree");
    assert_rest_copied(&copied);
    for name in ["locked", "unreadable.txt", "written.txt"] {
        assert!(!copied.join(name).exists(), "{name}");
    }
    h.close().await;
}

#[tokio::test]
async fn retry_copies_the_child_once_it_can_be_opened() {
    let temp = fixture();
    let tree = tree(temp.path(), &["locked/", "written.txt"]);
    let mut denied = Some(Unreadable::new(&tree.join("locked")));
    let mut writer = Some(written(&tree.join("written.txt")));
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&tree), &destination))
        .await;
    let first = failure(&mut h, 1).await;
    assert_eq!(first.source.as_deref(), Some(tree.join("locked").as_path()));
    // Retrying while the folder still cannot be listed asks again.
    decide(&h, &first, CopyChoice::Retry).await;
    let again = failure(&mut h, 1).await;
    assert!(again.version > first.version, "{again:?}");
    assert_eq!(again.source, first.source);
    drop(denied.take());
    decide(&h, &again, CopyChoice::Retry).await;
    let second = failure(&mut h, 1).await;
    assert_eq!(
        second.source.as_deref(),
        Some(tree.join("written.txt").as_path())
    );
    drop(writer.take());
    decide(&h, &second, CopyChoice::Retry).await;
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
        fs::read(copied.join("written.txt")).unwrap(),
        b"written.txt"
    );
    h.close().await;
}

#[tokio::test]
async fn skip_all_errors_leaves_out_later_children_denied_the_same_way_without_asking() {
    let temp = fixture();
    let names = ["first.txt", "second.txt", "third.txt"];
    let tree = tree(temp.path(), &names);
    let denied: Vec<_> = names
        .iter()
        .map(|name| Unreadable::new(&tree.join(name)))
        .collect();
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&tree), &destination))
        .await;
    let first = failure(&mut h, 1).await;
    decide(&h, &first, CopyChoice::SkipAllErrors).await;
    let done = finished(&mut h, 1).await;
    drop(denied);
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

/// Denies Everyone the deletion of `path` (and, for a folder, of its entries) until
/// dropped: a Move cannot take an item out of such a folder.
struct Undeletable(PathBuf);

impl Undeletable {
    fn new(path: &Path, rights: &str) -> Self {
        let output = icacls(path, &["/deny", &format!("*S-1-1-0:({rights})")]);
        assert!(output.status.success(), "fixture ACL: {output:?}");
        Self(path.to_owned())
    }
}

impl Drop for Undeletable {
    fn drop(&mut self) {
        let output = icacls(&self.0, &["/remove:d", "*S-1-1-0"]);
        if !output.status.success() {
            eprintln!("cannot restore fixture ACL: {output:?}");
        }
    }
}

/// A Move of `held.txt` out of a folder that denies the deletion of its entries.
async fn protected_move(temp: &Path) -> (Harness, PathBuf, PathBuf, Vec<Undeletable>) {
    let folder = temp.join("protected");
    fs::create_dir(&folder).unwrap();
    let held = folder.join("held.txt");
    fs::write(&held, b"held").unwrap();
    let destination = temp.join("destination");
    fs::create_dir(&destination).unwrap();
    let denied = vec![
        Undeletable::new(&held, "DE"),
        Undeletable::new(&folder, "DC"),
    ];
    let h = Harness::native();
    h.controller
        .enqueue_move(request(1, std::slice::from_ref(&held), &destination))
        .await;
    (h, held, destination, denied)
}

#[tokio::test]
async fn a_move_out_of_a_protected_folder_offers_skip_and_skip_leaves_the_item() {
    // T-E06: like every other transfer error, Retry, Skip and Skip all this error type.
    let temp = fixture();
    let (mut h, held, destination, denied) = protected_move(temp.path()).await;
    let failure = failure(&mut h, 1).await;
    assert!(failure.can_retry && failure.can_skip, "{failure:?}");
    assert_eq!(failure.stage, CopyErrorStage::Preparing, "{failure:?}");
    assert_eq!(failure.source.as_deref(), Some(held.as_path()));
    assert_eq!(
        failure.error.kind,
        io::ErrorKind::PermissionDenied,
        "{failure:?}"
    );
    // The question's text, for the acceptance check (shown with --nocapture).
    eprintln!("T-E06 question: {}", failure.error);
    decide(&h, &failure, CopyChoice::Skip).await;
    let done = h.terminal(1).await;
    drop(denied);
    assert_eq!(done.state, CopyOperationState::Failed, "{done:?}");
    assert_eq!((done.skipped, done.failed), (1, 1), "{done:?}");
    assert_eq!(fs::read(&held).unwrap(), b"held");
    assert!(!destination.join("held.txt").exists());
    h.close().await;
}

#[tokio::test]
async fn retry_moves_the_item_once_its_folder_lets_it_leave() {
    let temp = fixture();
    let (mut h, held, destination, denied) = protected_move(temp.path()).await;
    let failure = failure(&mut h, 1).await;
    drop(denied);
    decide(&h, &failure, CopyChoice::Retry).await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert!(!held.exists());
    assert_eq!(fs::read(destination.join("held.txt")).unwrap(), b"held");
    h.close().await;
}
