//! Links in Copy and Move on the native Linux filesystem: links kept as links,
//! followed links, links that cannot be followed, Move of the link itself, and
//! replacement of an existing name.
use super::*;
use std::os::unix::fs::symlink;

const LINKS: [&str; 4] = ["dangling", "to-file", "to-folder", "to-self"];

/// The acceptance fixture's links, beside the folder and the file they point to.
fn links_fixture(parent: &Path) -> PathBuf {
    let root = parent.join("fixture");
    fs::create_dir_all(root.join("deep/level-01/level-02")).unwrap();
    fs::write(root.join("deep/level-01/a.txt"), b"a").unwrap();
    fs::write(root.join("deep/level-01/level-02/b.txt"), b"bb").unwrap();
    fs::create_dir(root.join("text")).unwrap();
    fs::write(root.join("text/plain.txt"), b"plain text\n").unwrap();
    let links = root.join("links");
    fs::create_dir(&links).unwrap();
    symlink("../deep/level-01", links.join("to-folder")).unwrap();
    symlink("../text/plain.txt", links.join("to-file")).unwrap();
    symlink("../does-not-exist", links.join("dangling")).unwrap();
    symlink(".", links.join("to-self")).unwrap();
    links
}

fn new_folder(path: PathBuf) -> PathBuf {
    fs::create_dir(&path).unwrap();
    path
}

fn run(job: &mut worker::Job) {
    for _ in 0..10_000 {
        match job.step() {
            worker::Step::Continue => {}
            worker::Step::Done => return,
            worker::Step::Failure(failure) => panic!("unexpected failure: {failure:?}"),
            worker::Step::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
        }
    }
    panic!("the job did not finish within bounded steps");
}

fn next_conflict(job: &mut worker::Job) -> Arc<CopyConflict> {
    for _ in 0..10_000 {
        match job.step() {
            worker::Step::Continue => {}
            worker::Step::Conflict(conflict) => return conflict,
            worker::Step::Done => panic!("the job ended without a conflict"),
            worker::Step::Failure(failure) => panic!("unexpected failure: {failure:?}"),
        }
    }
    panic!("no conflict within bounded steps");
}

fn job(
    sources: &[PathBuf],
    destination: &Path,
    moving: bool,
    links: CopyLinkPolicy,
) -> worker::Job {
    let request = request(1, sources, destination);
    let input = if moving {
        CopyInput::move_paths(request.clone())
    } else {
        request.clone().into()
    };
    let status = Arc::new(Mutex::new(initial(&input)));
    let service = midnight_platform::LocalFileSystem::new(None);
    let metadata = TransferMetadataPolicy::ContentOnly;
    let cancel = Cancellation::default();
    let job = if moving {
        worker::Job::prepare_move_with_options(request, &service, cancel, status, metadata, links)
    } else {
        worker::Job::prepare_with_options(request, &service, cancel, status, metadata, links)
    }
    .unwrap();
    // A direct Job has no admission envelope: lend the Copy I/O permit link stages need.
    let permit = Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
    job.begin_io_step(Some(permit));
    job
}

fn tree(path: &Path) -> Vec<(PathBuf, String)> {
    let mut entries = Vec::new();
    let mut pending = vec![path.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            let relative = entry.path().strip_prefix(path).unwrap().to_owned();
            let fact = if kind.is_symlink() {
                format!("link {}", fs::read_link(entry.path()).unwrap().display())
            } else if kind.is_dir() {
                pending.push(entry.path());
                "folder".into()
            } else {
                format!("file {:?}", fs::read(entry.path()).unwrap())
            };
            entries.push((relative, fact));
        }
    }
    entries.sort();
    entries
}

#[test]
fn copy_without_following_recreates_every_link_with_its_exact_text() {
    let (_temp, parent) = protected_fixture();
    let links = links_fixture(&parent);
    let before = tree(&parent.join("fixture"));
    let destination = new_folder(parent.join("destination"));
    let sources: Vec<_> = LINKS.iter().map(|name| links.join(name)).collect();
    let mut copy = job(&sources, &destination, false, CopyLinkPolicy::Preserve);
    run(&mut copy);
    let done = copy.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!((done.files_completed, done.completed_roots), (4, 4));
    assert_eq!((done.failed, done.note_count), (0, 0));
    for name in LINKS {
        let copied = destination.join(name);
        assert!(
            fs::symlink_metadata(&copied)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(&copied).unwrap(),
            fs::read_link(links.join(name)).unwrap()
        );
    }
    // A folder holding links copies them as links too.
    let folder_copy = new_folder(parent.join("folder copy"));
    let mut copy = job(
        std::slice::from_ref(&links),
        &folder_copy,
        false,
        CopyLinkPolicy::Preserve,
    );
    run(&mut copy);
    assert_eq!(copy.finish().state, CopyOperationState::Completed);
    assert_eq!(tree(&folder_copy.join("links")), tree(&links));
    assert_eq!(
        tree(&parent.join("fixture")),
        before,
        "sources are unchanged"
    );
}

#[test]
fn copy_following_links_copies_targets_and_keeps_unfollowable_links_with_notes() {
    let (_temp, parent) = protected_fixture();
    let links = links_fixture(&parent);
    let destination = new_folder(parent.join("destination"));
    let sources: Vec<_> = LINKS.iter().map(|name| links.join(name)).collect();
    let mut copy = job(&sources, &destination, false, CopyLinkPolicy::FollowLinks);
    run(&mut copy);
    let done = copy.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.failed, 0, "{done:?}");
    // The folder link is followed and the file link copied as its content.
    assert_eq!(
        tree(&destination.join("to-folder")),
        tree(&parent.join("fixture/deep/level-01"))
    );
    assert_eq!(
        fs::read(destination.join("to-file")).unwrap(),
        b"plain text\n"
    );
    assert!(
        fs::symlink_metadata(destination.join("to-file"))
            .unwrap()
            .is_file()
    );
    // The dangling link stays a link, with a note.
    assert_eq!(
        fs::read_link(destination.join("dangling")).unwrap(),
        Path::new("../does-not-exist")
    );
    // Following the self link copies its folder once; the link inside it that
    // points back again is kept as a link.
    let copied_self = destination.join("to-self");
    assert!(fs::symlink_metadata(&copied_self).unwrap().is_dir());
    assert_eq!(
        fs::read_link(copied_self.join("to-self")).unwrap(),
        Path::new(".")
    );
    assert_eq!(
        fs::read(copied_self.join("to-file")).unwrap(),
        b"plain text\n"
    );
    assert_eq!(done.note_count, 3, "{:?}", done.notes);
    assert!(
        done.notes
            .iter()
            .any(|note| note.contains("dangling") && note.contains("does not exist")),
        "{:?}",
        done.notes
    );
    assert!(
        done.notes
            .iter()
            .any(|note| note.contains("to-self") && note.contains("contains it")),
        "{:?}",
        done.notes
    );
}

#[test]
fn copy_following_a_folder_that_links_to_itself_stops_with_the_link_kept() {
    let (_temp, parent) = protected_fixture();
    let cycle = new_folder(parent.join("cycle-only"));
    symlink(".", cycle.join("loop")).unwrap();
    let destination = new_folder(parent.join("destination"));
    let mut copy = job(
        std::slice::from_ref(&cycle),
        &destination,
        false,
        CopyLinkPolicy::FollowLinks,
    );
    run(&mut copy);
    let done = copy.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(
        tree(&destination.join("cycle-only")),
        [(PathBuf::from("loop"), "link .".to_owned())]
    );
    assert_eq!(done.note_count, 1);
    assert_eq!(fs::read_link(cycle.join("loop")).unwrap(), Path::new("."));
}

#[test]
fn move_moves_each_link_itself_and_leaves_its_target() {
    let (_temp, parent) = protected_fixture();
    let links = links_fixture(&parent);
    let expected: Vec<_> = LINKS
        .iter()
        .map(|name| fs::read_link(links.join(name)).unwrap())
        .collect();
    let deep = tree(&parent.join("fixture/deep"));
    let destination = new_folder(parent.join("destination"));
    let sources: Vec<_> = LINKS.iter().map(|name| links.join(name)).collect();
    let mut moving = job(&sources, &destination, true, CopyLinkPolicy::Preserve);
    run(&mut moving);
    let done = moving.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.native_entries_moved, 4);
    for (name, target) in LINKS.iter().zip(&expected) {
        assert_eq!(&fs::read_link(destination.join(name)).unwrap(), target);
        assert!(fs::symlink_metadata(links.join(name)).is_err(), "{name}");
    }
    assert_eq!(tree(&parent.join("fixture/deep")), deep);
    assert_eq!(
        fs::read(parent.join("fixture/text/plain.txt")).unwrap(),
        b"plain text\n"
    );
}

#[test]
fn copying_a_link_onto_an_existing_file_can_replace_it_and_keeps_the_original() {
    let (_temp, parent) = protected_fixture();
    let links = links_fixture(&parent);
    let destination = new_folder(parent.join("destination"));
    fs::write(destination.join("dangling"), b"existing").unwrap();
    let mut copy = job(
        &[links.join("dangling")],
        &destination,
        false,
        CopyLinkPolicy::Preserve,
    );
    let conflict = next_conflict(&mut copy);
    assert!(conflict.can_replace && !conflict.can_merge, "{conflict:?}");
    assert!(copy.accept_decision(CopyDecision {
        id: conflict.id,
        version: conflict.version,
        choice: CopyChoice::ReplaceRecoverable { all: false },
    }));
    run(&mut copy);
    let done = copy.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.recovery_count, 1);
    assert_eq!(fs::read(&done.recovery_paths[0]).unwrap(), b"existing");
    assert_eq!(
        fs::read_link(destination.join("dangling")).unwrap(),
        Path::new("../does-not-exist")
    );
}

#[test]
fn moving_a_link_onto_an_existing_file_asks_and_skip_keeps_both() {
    let (_temp, parent) = protected_fixture();
    let links = links_fixture(&parent);
    let destination = new_folder(parent.join("destination"));
    fs::write(destination.join("to-file"), b"existing").unwrap();
    let mut moving = job(
        &[links.join("to-file")],
        &destination,
        true,
        CopyLinkPolicy::Preserve,
    );
    let conflict = next_conflict(&mut moving);
    assert!(conflict.can_replace, "{conflict:?}");
    assert!(moving.accept_decision(CopyDecision {
        id: conflict.id,
        version: conflict.version,
        choice: CopyChoice::Skip,
    }));
    run(&mut moving);
    let done = moving.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.skipped, 1);
    assert_eq!(fs::read(destination.join("to-file")).unwrap(), b"existing");
    assert!(
        fs::symlink_metadata(links.join("to-file"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn moving_a_link_onto_an_existing_file_can_replace_it() {
    let (_temp, parent) = protected_fixture();
    let links = links_fixture(&parent);
    let destination = new_folder(parent.join("destination"));
    fs::write(destination.join("to-file"), b"existing").unwrap();
    let mut moving = job(
        &[links.join("to-file")],
        &destination,
        true,
        CopyLinkPolicy::Preserve,
    );
    let conflict = next_conflict(&mut moving);
    assert!(moving.accept_decision(CopyDecision {
        id: conflict.id,
        version: conflict.version,
        choice: CopyChoice::ReplaceRecoverable { all: false },
    }));
    run(&mut moving);
    let done = moving.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(
        fs::read_link(destination.join("to-file")).unwrap(),
        Path::new("../text/plain.txt")
    );
    assert!(fs::symlink_metadata(links.join("to-file")).is_err());
    assert_eq!(done.recovery_count, 1, "{done:?}");
    assert_eq!(fs::read(&done.recovery_paths[0]).unwrap(), b"existing");
}
