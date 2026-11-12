use super::*;
use midnight_core::{DiscoveryScope, DiscoveryState};

#[test]
fn recursive_discovery_precedes_writes_and_reports_exact_current_destinations() {
    let temp = fixture();
    let source = temp.path().join("source-中文");
    let destination = temp.path().join("destination-🧪");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("first.txt"), b"abc").unwrap();
    fs::write(source.join("nested/second.txt"), b"12345").unwrap();
    let request = request(1, std::slice::from_ref(&source), &destination);
    let status = Arc::new(Mutex::new(initial(&request.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job =
        worker::Job::prepare(request, &service, Cancellation::default(), status.clone()).unwrap();
    job.prepare_discovery();
    let scanned = snapshot(&status).discovery;
    assert_eq!(scanned.state, DiscoveryState::Complete);
    assert_eq!(scanned.scope, DiscoveryScope::Recursive);
    assert_eq!(
        (scanned.files, scanned.directories, scanned.bytes),
        (2, 2, Some(8))
    );
    assert_eq!(
        fs::read_dir(&destination).unwrap().count(),
        0,
        "discovery cannot create targets"
    );
    let mut saw_file_target = false;
    for _ in 0..100 {
        let step = job.step();
        let progress = snapshot(&status);
        if progress.current_file_size.is_some() {
            let source_path = progress.current_path.as_ref().expect("current source");
            let relative = source_path.strip_prefix(&source).unwrap();
            let target = destination.join(source.file_name().unwrap()).join(relative);
            assert_eq!(progress.current_destination, Some(target));
            saw_file_target = true;
        }
        match step {
            worker::Step::Done => {
                let done = job.finish();
                assert_eq!(done.state, CopyOperationState::Completed);
                assert_eq!(done.completed_roots, 1);
                assert_eq!(
                    (
                        done.files_completed,
                        done.completed_directories,
                        done.completed_bytes
                    ),
                    (2, 2, 8)
                );
                assert_eq!(done.discovery.state, DiscoveryState::Complete);
                assert_eq!(done.copied_bytes, 8);
                assert_eq!(done.verified_bytes, 8);
                assert!(saw_file_target);
                assert_eq!(fs::read(source.join("first.txt")).unwrap(), b"abc");
                assert_eq!(
                    fs::read(destination.join("source-中文/nested/second.txt")).unwrap(),
                    b"12345"
                );
                return;
            }
            worker::Step::Continue => {}
            _ => panic!("unexpected conflict or failure"),
        }
    }
    panic!("copy did not complete within bounded steps");
}

#[test]
fn same_volume_directory_move_keeps_root_scope_and_zero_content_io() {
    let (_temp, root) = moves::protected_fixture();
    let source = root.join("source");
    let destination = root.join("destination");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(
        source.join("nested/file"),
        b"must not be read for native rename",
    )
    .unwrap();
    let request = request(2, std::slice::from_ref(&source), &destination);
    let status = Arc::new(Mutex::new(initial(&request.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job =
        worker::Job::prepare_move(request, &service, Cancellation::default(), status.clone())
            .unwrap();
    job.prepare_discovery();
    let discovery = snapshot(&status).discovery;
    assert_eq!(discovery.state, DiscoveryState::Complete);
    assert_eq!(discovery.scope, DiscoveryScope::SelectedRoots);
    assert!(snapshot(&status).remaining_work.is_none());
    assert_eq!(
        (discovery.files, discovery.directories, discovery.bytes),
        (0, 1, None)
    );
    for _ in 0..20 {
        match job.step() {
            worker::Step::Done => {
                let done = job.finish();
                assert_eq!(done.state, CopyOperationState::Completed);
                assert_eq!(done.completed_roots, 1);
                assert_eq!(done.native_entries_moved, 1);
                assert_eq!(
                    (
                        done.copied_bytes,
                        done.verified_bytes,
                        done.validation_bytes
                    ),
                    (0, 0, 0)
                );
                assert!(!source.exists());
                assert!(destination.join("source/nested/file").exists());
                return;
            }
            worker::Step::Continue => {}
            _ => panic!("unexpected native move failure"),
        }
    }
    panic!("native move did not finish");
}

#[test]
fn changed_source_invalidates_discovered_totals_in_both_directions() {
    for grew in [false, true] {
        let temp = fixture();
        let source = temp.path().join("source");
        let destination = temp.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("first"), b"abc").unwrap();
        let request = request(3, std::slice::from_ref(&source), &destination);
        let status = Arc::new(Mutex::new(initial(&request.clone().into())));
        let service = midnight_platform::LocalFileSystem::new(None);
        let mut job =
            worker::Job::prepare(request, &service, Cancellation::default(), status.clone())
                .unwrap();
        job.prepare_discovery();
        if grew {
            fs::write(source.join("second"), b"more bytes").unwrap();
        } else {
            fs::remove_file(source.join("first")).unwrap();
        }
        let mut done = None;
        for _ in 0..100 {
            match job.step() {
                worker::Step::Continue => {}
                worker::Step::Done => {
                    done = Some(job.finish());
                    break;
                }
                _ => panic!("unexpected error while copying changed directory membership"),
            }
        }
        let done = done.expect("bounded completion");
        assert_eq!(done.state, CopyOperationState::Completed);
        assert_eq!(done.discovery.state, DiscoveryState::Unavailable);
        assert!(!done.discovery.message.is_empty());
        assert_eq!(done.files_completed, if grew { 2 } else { 0 });
        assert_eq!(done.completed_bytes, if grew { 13 } else { 0 });
    }
}

#[test]
fn same_volume_file_move_never_exposes_a_file_write_meter() {
    let (_temp, root) = moves::protected_fixture();
    let source = root.join("source-中文.txt");
    let destination = root.join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&source, b"native rename only").unwrap();
    let request = request(4, std::slice::from_ref(&source), &destination);
    let status = Arc::new(Mutex::new(initial(&request.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job =
        worker::Job::prepare_move(request, &service, Cancellation::default(), status.clone())
            .unwrap();
    let mut saw_current_item = false;
    for _ in 0..20 {
        let step = job.step();
        let progress = snapshot(&status);
        assert_eq!(progress.current_file_size, None);
        assert_eq!(progress.current_file_copied, 0);
        assert_eq!(progress.current_file_verified, 0);
        if let Some(target) = progress.current_destination {
            assert_eq!(progress.current_path.as_ref(), Some(&source));
            assert_eq!(target, destination.join(source.file_name().unwrap()));
            saw_current_item = true;
        }
        match step {
            worker::Step::Done => {
                let done = job.finish();
                assert_eq!(done.state, CopyOperationState::Completed);
                assert_eq!(done.native_entries_moved, 1);
                assert_eq!(done.copied_bytes, 0);
                assert!(
                    saw_current_item,
                    "native move retains source/target progress"
                );
                assert!(!source.exists());
                return;
            }
            worker::Step::Continue => {}
            _ => panic!("unexpected native file move failure"),
        }
    }
    panic!("native file move did not finish");
}

#[test]
fn overwrite_decision_preserves_paths_without_claiming_an_active_file_write() {
    let temp = fixture();
    let source = temp.path().join("source-🧪.txt");
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&source, b"source bytes").unwrap();
    let target = destination.join(source.file_name().unwrap());
    fs::write(&target, b"existing target").unwrap();
    let request = request(5, std::slice::from_ref(&source), &destination);
    let status = Arc::new(Mutex::new(initial(&request.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut job = worker::Job::prepare(request, &service, cancel.clone(), status.clone()).unwrap();
    for _ in 0..20 {
        let step = job.step();
        let progress = snapshot(&status);
        assert_eq!(progress.current_file_size, None);
        assert_eq!(progress.current_file_copied, 0);
        assert_eq!(progress.current_file_verified, 0);
        match step {
            worker::Step::Conflict(_) => {
                assert_eq!(progress.current_path.as_ref(), Some(&source));
                assert_eq!(progress.current_destination.as_ref(), Some(&target));
                assert_eq!(fs::read(&target).unwrap(), b"existing target");
                cancel.cancel();
                assert_eq!(job.finish().state, CopyOperationState::Cancelled);
                return;
            }
            worker::Step::Continue => {}
            _ => panic!("expected an overwrite decision before writing"),
        }
    }
    panic!("overwrite decision did not arrive");
}
