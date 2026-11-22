//! Links in Copy and Move on the native Windows filesystem: a junction kept as a junction
//! (created without privilege), followed into a copy of its folder, a Move that copies
//! what links point to and leaves the targets intact, the same Moves to another volume
//! (where a kept junction is recreated; these need NATIVE_MOVE_OTHER_VOLUME_ROOT and
//! print SKIP without it), dangling and cyclic junctions kept as links with their
//! reasons, and a relative directory symbolic link kept relative where this process may
//! create one (SKIP otherwise; the stage tests cover the junction in its place).
use super::*;
use std::os::windows::fs::MetadataExt;

const REPARSE_POINT: u32 = 0x400;

/// A junction `link` to the folder `target`, created by `mklink /J`, which needs no
/// privilege.
fn junction(link: &Path, target: &Path) {
    // `mklink` reads a `/` in a path as the start of a switch: name each folder with `\`.
    let native = |path: &Path| path.components().collect::<PathBuf>();
    let system =
        PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot")).join("System32");
    let output = std::process::Command::new(system.join("cmd.exe"))
        .args(["/d", "/c", "mklink", "/J"])
        .arg(native(link))
        .arg(native(target))
        .output()
        .unwrap();
    assert!(output.status.success(), "fixture junction: {output:?}");
}

fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_attributes() & REPARSE_POINT != 0)
}

/// The fixture: `data/file.txt`, and `tree/plain.txt` beside the junction `tree/j` to
/// `data`.
fn fixture_tree(parent: &Path) -> (PathBuf, PathBuf) {
    let data = parent.join("data");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("file.txt"), b"linked data").unwrap();
    let tree = parent.join("tree");
    fs::create_dir(&tree).unwrap();
    fs::write(tree.join("plain.txt"), b"plain").unwrap();
    junction(&tree.join("j"), &data);
    (data, tree)
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

fn identity(path: &Path) -> TransferIdentity {
    midnight_platform::LocalFileSystem::new(None)
        .prepare_transfer_source(path, &Cancellation::default())
        .unwrap()
        .entry()
        .identity
}

fn private_names(folder: &Path) -> Vec<std::ffi::OsString> {
    fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(".me-"))
        .collect()
}

/// A fresh folder below NATIVE_MOVE_OTHER_VOLUME_ROOT, an owned folder on another local
/// volume than `here` (as for the provider's cross-volume Move tests), or None after
/// printing SKIP.
fn other_volume(test: &str, here: &Path) -> Option<tempfile::TempDir> {
    let Some(root) = std::env::var_os("NATIVE_MOVE_OTHER_VOLUME_ROOT") else {
        eprintln!("SKIP {test}: NATIVE_MOVE_OTHER_VOLUME_ROOT names no folder on another volume");
        return None;
    };
    let other = tempfile::tempdir_in(root).unwrap();
    if identity(other.path()).device == identity(here).device {
        eprintln!("SKIP {test}: NATIVE_MOVE_OTHER_VOLUME_ROOT is on the same volume");
        return None;
    }
    Some(other)
}

#[test]
fn keeping_links_recreates_a_junction_without_privilege() {
    let temp = fixture();
    let (data, tree) = fixture_tree(temp.path());
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut copy = job(
        std::slice::from_ref(&tree),
        &destination,
        false,
        CopyLinkPolicy::Preserve,
    );
    run(&mut copy);
    let done = copy.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    let copied = destination.join("tree/j");
    assert!(is_link(&copied), "the junction stays a junction");
    assert_eq!(fs::read_link(&copied).unwrap(), data);
    // Through the recreated junction: the same folder, not a copy of it.
    assert_eq!(fs::read(copied.join("file.txt")).unwrap(), b"linked data");
    assert_eq!(
        fs::read(destination.join("tree/plain.txt")).unwrap(),
        b"plain"
    );
    assert!(private_names(&destination.join("tree")).is_empty());
    assert_eq!(done.note_count, 0, "{:?}", done.notes);
}

#[test]
fn copying_the_files_follows_a_junction_into_a_copy_of_its_folder() {
    let temp = fixture();
    let (data, tree) = fixture_tree(temp.path());
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut copy = job(
        std::slice::from_ref(&tree),
        &destination,
        false,
        CopyLinkPolicy::FollowLinks,
    );
    run(&mut copy);
    let done = copy.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    let copied = destination.join("tree/j");
    assert!(!is_link(&copied) && copied.is_dir());
    assert_eq!(fs::read(copied.join("file.txt")).unwrap(), b"linked data");
    assert_eq!(fs::read(data.join("file.txt")).unwrap(), b"linked data");
}

#[test]
fn a_move_copying_the_files_leaves_what_links_point_to_intact() {
    let temp = fixture();
    let (data, tree) = fixture_tree(temp.path());
    let before = identity(&data.join("file.txt"));
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut moving = job(
        std::slice::from_ref(&tree),
        &destination,
        true,
        CopyLinkPolicy::FollowLinks,
    );
    run(&mut moving);
    let done = moving.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    // The folder was traversed, not renamed whole: its junction became a real folder.
    let moved = destination.join("tree/j");
    assert!(!is_link(&moved) && moved.is_dir());
    assert_eq!(fs::read(moved.join("file.txt")).unwrap(), b"linked data");
    assert_eq!(
        fs::read(destination.join("tree/plain.txt")).unwrap(),
        b"plain"
    );
    assert!(!tree.exists(), "the moved folder and its junction are gone");
    // The folder the junction pointed to: the same file, the same bytes.
    assert_eq!(fs::read(data.join("file.txt")).unwrap(), b"linked data");
    assert_eq!(identity(&data.join("file.txt")), before);
}

#[test]
fn a_move_keeping_links_moves_the_junction_itself() {
    let temp = fixture();
    let (data, tree) = fixture_tree(temp.path());
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut moving = job(
        &[tree.join("j")],
        &destination,
        true,
        CopyLinkPolicy::Preserve,
    );
    run(&mut moving);
    let done = moving.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.native_entries_moved, 1);
    assert!(is_link(&destination.join("j")));
    assert_eq!(fs::read_link(destination.join("j")).unwrap(), data);
    assert!(!tree.join("j").exists());
    assert_eq!(fs::read(data.join("file.txt")).unwrap(), b"linked data");
}

#[test]
fn a_move_keeping_links_to_another_volume_recreates_the_junction_and_removes_the_original() {
    let temp = fixture();
    let Some(other) = other_volume("a junction moved to another volume", temp.path()) else {
        return;
    };
    let (data, tree) = fixture_tree(temp.path());
    let before = identity(&data.join("file.txt"));
    let mut moving = job(
        &[tree.join("j")],
        other.path(),
        true,
        CopyLinkPolicy::Preserve,
    );
    run(&mut moving);
    let done = moving.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    let moved = other.path().join("j");
    assert!(is_link(&moved), "the junction was recreated as a junction");
    assert_eq!(fs::read_link(&moved).unwrap(), data);
    assert!(
        fs::symlink_metadata(tree.join("j")).is_err(),
        "the original junction is gone"
    );
    assert!(private_names(other.path()).is_empty());
    // What the junction pointed to: the same file, the same bytes.
    assert_eq!(fs::read(data.join("file.txt")).unwrap(), b"linked data");
    assert_eq!(identity(&data.join("file.txt")), before);
    assert_eq!(done.note_count, 0, "{:?}", done.notes);
    fs::remove_dir(&moved).unwrap();
}

#[test]
fn a_move_copying_the_files_to_another_volume_leaves_what_links_point_to_intact() {
    // Copying across volumes removes each verified source through a recovery stage beside
    // it, which a Move takes only in a protected folder.
    let (_temp, parent) = protected_fixture();
    let Some(other) = other_volume("a folder with a junction moved to another volume", &parent)
    else {
        return;
    };
    let (data, tree) = fixture_tree(&parent);
    let before = identity(&data.join("file.txt"));
    let mut moving = job(
        std::slice::from_ref(&tree),
        other.path(),
        true,
        CopyLinkPolicy::FollowLinks,
    );
    run(&mut moving);
    let done = moving.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    let moved = other.path().join("tree/j");
    assert!(!is_link(&moved) && moved.is_dir());
    assert_eq!(fs::read(moved.join("file.txt")).unwrap(), b"linked data");
    assert_eq!(
        fs::read(other.path().join("tree/plain.txt")).unwrap(),
        b"plain"
    );
    assert!(!tree.exists(), "the moved folder and its junction are gone");
    assert_eq!(fs::read(data.join("file.txt")).unwrap(), b"linked data");
    assert_eq!(identity(&data.join("file.txt")), before);
}

#[test]
fn dangling_and_cyclic_junctions_are_kept_as_links_with_their_reasons() {
    let temp = fixture();
    let tree = temp.path().join("tree");
    fs::create_dir(&tree).unwrap();
    let missing = temp.path().join("missing");
    fs::create_dir(&missing).unwrap();
    junction(&tree.join("dangling"), &missing);
    fs::remove_dir(&missing).unwrap();
    junction(&tree.join("loop"), &tree);
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut copy = job(
        std::slice::from_ref(&tree),
        &destination,
        false,
        CopyLinkPolicy::FollowLinks,
    );
    run(&mut copy);
    let done = copy.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert!(is_link(&destination.join("tree/dangling")));
    assert!(is_link(&destination.join("tree/loop")));
    assert_eq!(done.note_count, 2, "{:?}", done.notes);
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
            .any(|note| note.contains("loop") && note.contains("contains it")),
        "{:?}",
        done.notes
    );
    assert!(done.notes.iter().all(|note| !note.contains("os error")));
}

#[test]
fn a_relative_directory_symbolic_link_stays_relative_or_becomes_a_junction_with_a_note() {
    let temp = fixture();
    let tree = temp.path().join("tree");
    fs::create_dir_all(tree.join("sub")).unwrap();
    fs::write(tree.join("sub/file.txt"), b"sub").unwrap();
    if std::os::windows::fs::symlink_dir(r"sub", tree.join("rel")).is_err() {
        // Creating the fixture needs the rights whose absence the copy is about: without
        // them there is no symbolic link to copy (the stage tests cover the fallback).
        eprintln!(
            "SKIP a relative directory symbolic link: this process may not create one \
             (neither Developer Mode nor administrator rights)"
        );
        return;
    }
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut copy = job(
        std::slice::from_ref(&tree),
        &destination,
        false,
        CopyLinkPolicy::Preserve,
    );
    run(&mut copy);
    let done = copy.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    let copied = destination.join("tree/rel");
    assert!(is_link(&copied));
    // The fixture could be created, so this process may create symbolic links too.
    assert_eq!(fs::read_link(&copied).unwrap(), Path::new("sub"));
    assert_eq!(fs::read(copied.join("file.txt")).unwrap(), b"sub");
    assert_eq!(done.note_count, 0, "{:?}", done.notes);
}
