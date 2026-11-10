use super::*;

fn prepare_move(
    paths: &[PathBuf],
    destination: &Path,
    cancel: Cancellation,
) -> (worker::Job, Status) {
    let request = request(1, paths, destination);
    let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
    let service = CrossDeviceService(midnight_platform::LocalFileSystem::new(None));
    let job = worker::Job::prepare_move(request, &service, cancel, status.clone()).unwrap();
    (job, status)
}

fn drive(job: &mut worker::Job) {
    for _ in 0..1000 {
        match job.step() {
            worker::Step::Continue => {}
            worker::Step::Done => return,
            worker::Step::Failure(failure) => decide_failure(job, &failure),
            worker::Step::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
        }
    }
    panic!("Move did not finish within bounded fixture steps");
}

#[test]
fn linux_move_refuses_an_existing_writer_before_any_publication() {
    let (_temp, parent) = protected_fixture();
    let path = parent.join("source");
    let destination = parent.join("destination");
    fs::write(&path, b"original").unwrap();
    let _writer = fs::File::options().write(true).open(&path).unwrap();
    fs::create_dir(&destination).unwrap();
    let request = request(1, std::slice::from_ref(&path), &destination);
    let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
    let service = CrossDeviceService(midnight_platform::LocalFileSystem::new(None));
    let mut job =
        worker::Job::prepare_move(request, &service, Cancellation::default(), status).unwrap();
    drive(&mut job);
    let done = job.finish();
    assert!(
        done.failed > 0,
        "an existing source writer must prevent cross-volume publication"
    );
    assert_eq!(fs::read(path).unwrap(), b"original");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}

#[test]
fn linux_move_finishes_each_verified_file_before_starting_the_next_and_cancel_retains_rest() {
    use std::os::unix::ffi::OsStringExt;
    let (_temp, parent) = protected_fixture();
    let paths = [
        parent.join("first 文档"),
        parent.join(std::ffi::OsString::from_vec(b"second-\xff".to_vec())),
    ];
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    for path in &paths {
        fs::write(path, vec![42; 600_000]).unwrap();
    }
    let cancel = Cancellation::default();
    let (mut job, status) = prepare_move(&paths, &destination, cancel.clone());
    for _ in 0..1000 {
        assert!(matches!(job.step(), worker::Step::Continue));
        if snapshot(&status).source_files_removed == 1 {
            break;
        }
    }
    assert_eq!(snapshot(&status).source_files_removed, 1);
    assert!(!paths[0].exists());
    assert!(!destination.join(paths[1].file_name().unwrap()).exists());
    assert_eq!(
        fs::read(destination.join(paths[0].file_name().unwrap())).unwrap(),
        vec![42; 600_000]
    );
    cancel.cancel();
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Cancelled, "{done:?}");
    assert_eq!(done.source_files_removed, 1);
    assert_eq!(done.source_residual_count, 0);
    assert_eq!(fs::read(&paths[1]).unwrap(), vec![42; 600_000]);
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 3);
}

#[test]
fn linux_move_changed_destination_after_publication_keeps_source_and_later_files() {
    let (_temp, parent) = protected_fixture();
    let paths = [parent.join("first"), parent.join("second")];
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    for path in &paths {
        fs::write(path, b"original").unwrap();
    }
    let (mut job, status) = prepare_move(&paths, &destination, Cancellation::default());
    for _ in 0..1000 {
        assert!(matches!(job.step(), worker::Step::Continue));
        if snapshot(&status).files_completed == 1 {
            break;
        }
    }
    fs::write(destination.join("first"), b"changed!").unwrap();
    let mut decided = false;
    for _ in 0..1000 {
        match job.step() {
            worker::Step::Continue => {}
            worker::Step::Failure(failure) => {
                assert_eq!(failure.stage, CopyErrorStage::Verifying);
                assert!(
                    !failure.can_retry,
                    "changed bytes cannot authorize deletion"
                );
                assert!(job.accept_decision(CopyDecision {
                    id: failure.id,
                    version: failure.version,
                    choice: CopyChoice::Cancel,
                }));
                decided = true;
                break;
            }
            _ => panic!("changed destination must require a failure decision"),
        }
    }
    assert!(decided);
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.source_files_removed, 0, "{done:?}");
    assert_eq!(done.source_residual_count, 0);
    assert_eq!(done.state, CopyOperationState::Cancelled);
    for path in paths {
        assert_eq!(fs::read(path).unwrap(), b"original");
    }
    assert!(!destination.join("second").exists());
}

#[test]
fn linux_move_cancellation_after_capture_restores_original_and_removes_empty_stage() {
    let (_temp, parent) = protected_fixture();
    let path = parent.join("source");
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    let (mut job, status) = prepare_move(std::slice::from_ref(&path), &destination, cancel.clone());
    for _ in 0..1000 {
        assert!(matches!(job.step(), worker::Step::Continue));
        if snapshot(&status).source_names_staged == 1 {
            break;
        }
    }
    assert_eq!(snapshot(&status).source_names_staged, 1);
    cancel.cancel();
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.source_files_removed, 0, "{done:?}");
    assert_eq!(done.source_residual_count, 0);
    assert_eq!(fs::read(path).unwrap(), b"original");
    assert_eq!(fs::read_dir(parent).unwrap().count(), 3);
}

#[test]
fn linux_raw_name_move_with_required_time_completes_without_any_source_residual() {
    use std::{
        fs::FileTimes,
        os::unix::ffi::OsStringExt,
        time::{Duration, SystemTime},
    };
    let (_temp, parent) = protected_fixture();
    let path = parent.join(std::ffi::OsString::from_vec(b"source-\xff".to_vec()));
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&path, b"verified data").unwrap();
    let time = SystemTime::UNIX_EPOCH + Duration::new(12345, 123456789);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(time))
        .unwrap();
    let request = request(1, std::slice::from_ref(&path), &destination);
    let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job = worker::Job::prepare_move_with_metadata(
        request,
        &service,
        Cancellation::default(),
        status,
        TransferMetadataPolicy::RequireModifiedTime,
    )
    .unwrap();
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.source_files_removed, 0);
    assert_eq!(done.native_entries_moved, 1);
    assert_eq!(done.source_residual_count, 0);
    assert_eq!(
        done.copied_bytes + done.verified_bytes + done.validation_bytes,
        0
    );
    assert!(!path.exists());
    let published = destination.join(path.file_name().unwrap());
    assert_eq!(fs::read(&published).unwrap(), b"verified data");
    assert_eq!(fs::metadata(published).unwrap().modified().unwrap(), time);
    assert_eq!(fs::read_dir(parent).unwrap().count(), 2);
}
