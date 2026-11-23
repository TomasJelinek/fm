//! Native recursive Move acceptance; all paths belong to private fixtures.
use super::*;
#[path = "move_directory_retry_tests.rs"]
mod retry;

pub(in super::super) fn tree_fixture() -> (tempfile::TempDir, PathBuf) {
    let (temp, parent) = protected_fixture();
    #[cfg(windows)]
    inheritable_user_access(&parent);
    (temp, parent)
}

/// The production temporary stage's private ACE is intentionally not inheritable.
/// Give only this owned fixture's token-user ACE inheritance so ordinary child
/// directories also meet the native Move parent policy. `icacls /grant:r` replaces
/// that user's explicit grant like `SetAccessRule`, without starting PowerShell
/// (about 40 ms instead of 0.7-1.2 s per fixture); paths stay literal arguments.
#[cfg(windows)]
fn inheritable_user_access(path: &Path) {
    static USER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let system =
        PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot")).join("System32");
    let user = USER.get_or_init(|| {
        let output = std::process::Command::new(system.join("whoami.exe"))
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .unwrap();
        assert!(output.status.success(), "whoami: {output:?}");
        // "account","S-1-..."; only the ASCII SID field is used.
        let text = String::from_utf8_lossy(&output.stdout);
        let sid = text.trim().rsplit(',').next().unwrap_or_default();
        let sid = sid.trim_matches('"').to_owned();
        assert!(sid.starts_with("S-1-"), "token user SID: {text:?}");
        sid
    });
    let output = std::process::Command::new(system.join("icacls.exe"))
        .arg(path)
        .args(["/grant:r", &format!("*{user}:(OI)(CI)F")])
        .output()
        .unwrap();
    assert!(output.status.success(), "fixture ACL: {output:?}");
}

fn prepare(paths: &[PathBuf], destination: &Path, cancel: Cancellation) -> (worker::Job, Status) {
    let request = request(1, paths, destination);
    let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
    let provider = CrossDeviceService(midnight_platform::LocalFileSystem::new(None));
    let job = worker::Job::prepare_move(request, &provider, cancel, status.clone()).unwrap();
    (job, status)
}

fn drive(job: &mut worker::Job) {
    for _ in 0..10000 {
        match job.step() {
            worker::Step::Continue => {}
            worker::Step::Done => return,
            worker::Step::Failure(failure) => decide_failure(job, &failure),
            worker::Step::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
        }
    }
    panic!("recursive Move exceeded fixture step budget");
}

#[test]
fn recursive_move_nested_and_empty_directories_complete() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree 文档");
    let destination = parent.join("destination");
    fs::create_dir_all(source.join("nested/empty")).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("one"), b"first").unwrap();
    fs::write(source.join("nested/two"), b"second").unwrap();
    let (mut job, _) = prepare(
        std::slice::from_ref(&source),
        &destination,
        Cancellation::default(),
    );
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.source_files_removed, 2);
    assert!(!source.exists());
    let copied = destination.join("tree 文档");
    assert_eq!(fs::read(copied.join("one")).unwrap(), b"first");
    assert_eq!(fs::read(copied.join("nested/two")).unwrap(), b"second");
    assert!(copied.join("nested/empty").is_dir());
}

#[test]
fn recursive_move_cancellation_keeps_unfinished_children_and_parent() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    let destination = parent.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    for name in ["one", "two", "three"] {
        fs::write(source.join(name), vec![42; 400_000]).unwrap();
    }
    let cancel = Cancellation::default();
    let (mut job, status) = prepare(std::slice::from_ref(&source), &destination, cancel.clone());
    for _ in 0..10000 {
        assert!(matches!(job.step(), worker::Step::Continue));
        if snapshot(&status).source_files_removed == 1 {
            break;
        }
    }
    assert_eq!(snapshot(&status).source_files_removed, 1);
    assert_eq!(fs::read_dir(&source).unwrap().count(), 2);
    assert_eq!(fs::read_dir(destination.join("tree")).unwrap().count(), 1);
    cancel.cancel();
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Cancelled, "{done:?}");
    assert_eq!(done.source_files_removed, 1);
    assert_eq!(done.source_residual_count, 0);
    assert_eq!(fs::read_dir(source).unwrap().count(), 2);
}

#[test]
fn recursive_move_merge_skip_retains_skipped_child_and_ancestors() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    let destination = parent.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir_all(destination.join("tree")).unwrap();
    fs::write(source.join("keep"), b"source").unwrap();
    fs::write(source.join("move"), b"move").unwrap();
    fs::write(destination.join("tree/keep"), b"winner").unwrap();
    let (mut job, _) = prepare(
        std::slice::from_ref(&source),
        &destination,
        Cancellation::default(),
    );
    for _ in 0..10000 {
        match job.step() {
            worker::Step::Done => break,
            worker::Step::Continue => {}
            worker::Step::Failure(failure) => decide_failure(&mut job, &failure),
            worker::Step::Conflict(conflict) => {
                let choice = if conflict.source.kind == EntryKind::Directory {
                    CopyChoice::Merge { all: false }
                } else {
                    CopyChoice::Skip
                };
                assert!(job.accept_decision(CopyDecision {
                    id: conflict.id,
                    version: conflict.version,
                    choice
                }));
            }
        }
    }
    let done = job.finish();
    // The skipped file was the user's choice, not an error.
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.skipped, 1, "{done:?}");
    assert_eq!(done.source_files_removed, 1);
    assert_eq!(
        done.failed, 0,
        "a deliberate skip is not a removal error: {done:?}"
    );
    assert_eq!(fs::read(source.join("keep")).unwrap(), b"source");
    assert!(!source.join("move").exists());
    assert_eq!(fs::read(destination.join("tree/keep")).unwrap(), b"winner");
}

/// Children that gain another name (a hard link) while the Move runs: Windows moves them
/// as Delete removes such a file, taking only the moved name, and the other names keep
/// the data; Linux removes only single-link files, so they stop without undoing the
/// completed file.
#[test]
fn recursive_move_later_hardlinked_child_keeps_the_data_under_its_other_name() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    let destination = parent.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    for name in ["one", "two", "three"] {
        fs::write(source.join(name), b"original").unwrap();
    }
    let (mut job, status) = prepare(
        std::slice::from_ref(&source),
        &destination,
        Cancellation::default(),
    );
    for _ in 0..10000 {
        assert!(matches!(job.step(), worker::Step::Continue));
        if snapshot(&status).source_files_removed == 1 {
            break;
        }
    }
    let remaining: Vec<_> = fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(remaining.len(), 2);
    // Both unfinished children become inadmissible, independently of native order.
    for (index, path) in remaining.iter().enumerate() {
        fs::hard_link(path, parent.join(format!("alias-{index}"))).unwrap();
    }
    drive(&mut job);
    let done = job.finish();
    if cfg!(windows) {
        assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
        assert_eq!(done.source_files_removed, 3);
        assert!(!source.exists());
        for index in 0..remaining.len() {
            assert_eq!(
                fs::read(parent.join(format!("alias-{index}"))).unwrap(),
                b"original"
            );
        }
        assert_eq!(fs::read_dir(destination.join("tree")).unwrap().count(), 3);
        return;
    }
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!(done.source_files_removed, 1);
    assert_eq!(done.files_completed, 1);
    assert_eq!(done.source_residual_count, 0);
    for path in remaining {
        assert_eq!(fs::read(path).unwrap(), b"original");
    }
    assert_eq!(fs::read_dir(destination.join("tree")).unwrap().count(), 1);
}

#[test]
fn recursive_move_native_directory_removal_refuses_unexpected_children() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    fs::create_dir(&source).unwrap();
    let provider = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut prepared = provider.prepare_move_source(&source, &cancel).unwrap();
    prepared.capture.preflight_finalization(&cancel).unwrap();
    assert!(prepared.source.next_move_child(&cancel).unwrap().is_none());
    fs::write(source.join("unexpected"), b"keep").unwrap();
    assert!(prepared.capture.remove_empty_directory(&cancel).is_err());
    drop(prepared);
    assert_eq!(fs::read(source.join("unexpected")).unwrap(), b"keep");
}

#[test]
fn recursive_move_copy_directory_has_no_child_mutation_authority() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), b"keep").unwrap();
    let provider = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut copy = provider.prepare_transfer_source(&source, &cancel).unwrap();
    assert!(copy.next_move_child(&cancel).is_err());
    drop(copy);
    assert_eq!(fs::read(source.join("file")).unwrap(), b"keep");
}

#[test]
fn recursive_move_destination_inside_source_is_refused_before_creation() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    let destination = source.join("inside");
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("file"), b"keep").unwrap();
    let request = request(1, std::slice::from_ref(&source), &destination);
    let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
    let provider = midnight_platform::LocalFileSystem::new(None);
    assert!(
        worker::Job::prepare_move(request, &provider, Cancellation::default(), status).is_err()
    );
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    assert_eq!(fs::read(source.join("file")).unwrap(), b"keep");
}

/// A Move has no fixed depth limit, as Copy has none: a tree deeper than the 128 folders
/// it once stopped at moves whole. (On Linux the folders it is inside count against the
/// transfers' descriptor budget, which a test process with the usual 1,024 descriptors
/// sets at 85 levels; the depth there stays within it.)
#[test]
fn recursive_move_of_a_tree_deeper_than_128_folders_moves_it_whole() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    let destination = parent.join("destination");
    let depth = if cfg!(windows) { 130 } else { 80 };
    let deepest = (0..depth).fold(source.clone(), |path, _| path.join("d"));
    fs::create_dir_all(&deepest).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(deepest.join("moved"), b"keep").unwrap();
    let (mut job, _) = prepare(
        std::slice::from_ref(&source),
        &destination,
        Cancellation::default(),
    );
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.source_files_removed, 1);
    let moved = (0..depth).fold(destination.join("tree"), |path, _| path.join("d"));
    assert_eq!(fs::read(moved.join("moved")).unwrap(), b"keep");
    assert!(!source.exists());
}

#[test]
fn recursive_move_overlapping_file_root_is_refused_before_destination_creation() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    let destination = parent.join("destination");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&destination).unwrap();
    let file = source.join("nested/file");
    fs::write(&file, b"retained").unwrap();
    let provider = midnight_platform::LocalFileSystem::new(None);
    for paths in [
        vec![source.clone(), file.clone()],
        vec![file.clone(), source.clone()],
    ] {
        let request = request(1, &paths, &destination);
        let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
        assert!(
            worker::Job::prepare_move(request, &provider, Cancellation::default(), status).is_err()
        );
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        assert_eq!(fs::read(&file).unwrap(), b"retained");
    }
}

#[test]
fn recursive_move_crosses_native_enumeration_pages_without_losing_children() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    let destination = parent.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    for index in 0..192 {
        fs::write(
            source.join(format!("{index:03}-{}", "n".repeat(176))),
            index.to_string(),
        )
        .unwrap();
    }
    let (mut job, _) = prepare(
        std::slice::from_ref(&source),
        &destination,
        Cancellation::default(),
    );
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.source_files_removed, 192);
    assert!(!source.exists());
    for index in 0..192 {
        assert_eq!(
            fs::read(
                destination
                    .join("tree")
                    .join(format!("{index:03}-{}", "n".repeat(176)))
            )
            .unwrap(),
            index.to_string().as_bytes()
        );
    }
}

#[test]
fn recursive_move_merge_replacement_preserves_old_target_and_unrelated_children() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    let destination = parent.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir_all(destination.join("tree")).unwrap();
    fs::write(source.join("replace"), b"new").unwrap();
    fs::write(destination.join("tree/replace"), b"old").unwrap();
    fs::write(destination.join("tree/unrelated"), b"keep").unwrap();
    let (mut job, _) = prepare(
        std::slice::from_ref(&source),
        &destination,
        Cancellation::default(),
    );
    for _ in 0..10000 {
        match job.step() {
            worker::Step::Done => break,
            worker::Step::Continue => {}
            worker::Step::Failure(failure) => decide_failure(&mut job, &failure),
            worker::Step::Conflict(conflict) => {
                let choice = if conflict.source.kind == EntryKind::Directory {
                    CopyChoice::Merge { all: false }
                } else {
                    CopyChoice::ReplaceRecoverable { all: false }
                };
                assert!(job.accept_decision(CopyDecision {
                    id: conflict.id,
                    version: conflict.version,
                    choice
                }));
            }
        }
    }
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.source_files_removed, 1);
    assert!(!source.exists());
    assert_eq!(fs::read(destination.join("tree/replace")).unwrap(), b"new");
    assert_eq!(
        fs::read(destination.join("tree/unrelated")).unwrap(),
        b"keep"
    );
    assert_eq!(done.recovery_paths.len(), 1);
    assert_eq!(fs::read(&done.recovery_paths[0]).unwrap(), b"old");
}

#[test]
fn recursive_move_renamed_directory_uses_original_source_authority() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    let destination = parent.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir_all(destination.join("tree")).unwrap();
    fs::write(source.join("file"), b"new").unwrap();
    let (mut job, _) = prepare(
        std::slice::from_ref(&source),
        &destination,
        Cancellation::default(),
    );
    for _ in 0..10000 {
        match job.step() {
            worker::Step::Done => break,
            worker::Step::Continue => {}
            worker::Step::Failure(failure) => decide_failure(&mut job, &failure),
            worker::Step::Conflict(conflict) => {
                assert!(job.accept_decision(CopyDecision {
                    id: conflict.id,
                    version: conflict.version,
                    choice: CopyChoice::Rename("renamed".into())
                }));
            }
        }
    }
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert!(!source.exists());
    assert_eq!(fs::read(destination.join("renamed/file")).unwrap(), b"new");
    assert_eq!(fs::read_dir(destination.join("tree")).unwrap().count(), 0);
}

#[test]
fn recursive_move_long_paths_and_raw_names_preserve_content() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("source-文档".repeat(12));
    let nested = "nested-目录".repeat(12);
    let destination = parent.join("destination");
    fs::create_dir_all(source.join(&nested)).unwrap();
    fs::create_dir(&destination).unwrap();
    #[cfg(windows)]
    let raw = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[b'r' as u16, 0xd800, b'w' as u16])
    };
    #[cfg(target_os = "linux")]
    let raw = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(b"raw-\xff".to_vec())
    };
    fs::write(source.join(&nested).join(&raw), b"exact raw path bytes").unwrap();
    let (mut job, _) = prepare(
        std::slice::from_ref(&source),
        &destination,
        Cancellation::default(),
    );
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert!(!source.exists());
    assert_eq!(
        fs::read(
            destination
                .join(source.file_name().unwrap())
                .join(nested)
                .join(raw)
        )
        .unwrap(),
        b"exact raw path bytes"
    );
}

#[cfg(windows)]
#[test]
fn recursive_move_directory_streams_are_refused_before_publication() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    let destination = parent.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    let mut stream = source.as_os_str().to_owned();
    stream.push(":unverified");
    fs::write(&stream, b"directory stream").unwrap();
    let request = request(1, std::slice::from_ref(&source), &destination);
    let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
    let provider = CrossDeviceService(midnight_platform::LocalFileSystem::new(None));
    let mut job =
        worker::Job::prepare_move(request, &provider, Cancellation::default(), status).unwrap();
    drive(&mut job);
    let done = job.finish();
    assert!(done.failed > 0, "{done:?}");
    assert_eq!(done.directories_created, 0);
    assert_eq!(fs::read(stream).unwrap(), b"directory stream");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn recursive_move_link_child_does_not_follow_or_delete_referent() {
    use std::os::unix::fs::symlink;
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    let destination = parent.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(parent.join("referent"), b"keep").unwrap();
    symlink(parent.join("referent"), source.join("link")).unwrap();
    let (mut job, _) = prepare(
        std::slice::from_ref(&source),
        &destination,
        Cancellation::default(),
    );
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.failed, 1, "{done:?}");
    assert_eq!(done.source_files_removed, 0);
    assert_eq!(fs::read(parent.join("referent")).unwrap(), b"keep");
    assert!(
        fs::symlink_metadata(source.join("link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn recursive_move_directory_substitution_retains_both_objects() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("tree");
    fs::create_dir(&source).unwrap();
    let provider = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = provider.prepare_move_source(&source, &cancel).unwrap();
    fs::rename(&source, parent.join("original")).unwrap();
    fs::create_dir(&source).unwrap();
    assert!(prepared.capture.remove_empty_directory(&cancel).is_err());
    drop(prepared);
    assert!(source.is_dir());
    assert!(parent.join("original").is_dir());
}
