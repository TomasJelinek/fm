//! The first Move increment accepts bounded ordinary file roots only.
use super::*;
use std::ffi::OsStr;
#[cfg(target_os = "linux")]
#[path = "move_linux_tests.rs"]
mod linux;
#[path = "copy_metadata_tests.rs"]
mod metadata;

fn protected_fixture() -> (tempfile::TempDir, PathBuf) {
    use std::io::Write;
    let temp = fixture();
    let provider = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    fs::write(temp.path().join("seed"), b"seed").unwrap();
    let destination = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let target = destination
        .prepare_target(OsStr::new("seed"), &cancel)
        .unwrap()
        .unwrap();
    let mut stage = destination
        .stage_file_for_target(
            OsStr::new("seed"),
            target,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    stage.write_all(b"fixture replacement").unwrap();
    let TransferPublication::Replaced { recovery, .. } =
        stage.publish_replace(OsStr::new("seed"), &cancel).unwrap()
    else {
        panic!("fixture recovery creation failed");
    };
    let parent = recovery.path.parent().unwrap().to_owned();
    drop(stage);
    (temp, parent)
}

fn prepare_move(
    sources: &[PathBuf],
    destination: &Path,
    cancel: Cancellation,
) -> (worker::Job, Status) {
    let request = request(1, sources, destination);
    let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
    let service = midnight_platform::LocalFileSystem::new(None);
    let job = worker::Job::prepare_move(request, &service, cancel, status.clone()).unwrap();
    (job, status)
}

fn drive(job: &mut worker::Job) {
    for _ in 0..1000 {
        match job.step() {
            worker::Step::Continue => {}
            worker::Step::Done => return,
            worker::Step::Conflict(value) => panic!("unexpected conflict: {value:?}"),
        }
    }
    panic!("Move failed to finish within its bounded fixture steps");
}

#[test]
fn move_retains_originals_only_after_all_files_publish_and_validate() {
    let (_temp, parent) = protected_fixture();
    let paths = [parent.join("first 文档"), parent.join("second 🦀")];
    for (index, path) in paths.iter().enumerate() {
        fs::write(path, vec![42 + index as u8; 600_000]).unwrap();
    }
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    let (mut job, status) = prepare_move(&paths, &destination, Cancellation::default());
    for _ in 0..1000 {
        assert!(
            paths.iter().all(|path| path.exists()),
            "sources must remain until whole-batch barrier"
        );
        assert!(matches!(job.step(), worker::Step::Continue));
        if snapshot(&status).phase == CopyOperationPhase::ValidatingBeforeMove {
            break;
        }
    }
    assert_eq!(snapshot(&status).files_completed, 2);
    assert_eq!(snapshot(&status).source_names_captured, 0);
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.intent, TransferIntent::RecoverableMove);
    assert_eq!(done.source_names_captured, 2);
    assert_eq!(done.source_recovery_count, 2);
    assert_eq!(done.validation_bytes, 4_800_000);
    assert!(paths.iter().all(|path| !path.exists()));
    for retained in done.source_recoveries {
        assert!(retained.matched_source);
        assert_eq!(
            retained.recovery.path.file_name(),
            retained.source.file_name()
        );
        assert_eq!(
            fs::read(&retained.recovery.path).unwrap(),
            fs::read(destination.join(retained.source.file_name().unwrap())).unwrap()
        );
    }
}

#[test]
fn move_rejects_mixed_roots_and_proof_budget_before_any_publication() {
    let (_temp, parent) = protected_fixture();
    let file = parent.join("file");
    let directory = parent.join("directory");
    let destination = parent.join("destination");
    fs::write(&file, b"original").unwrap();
    fs::create_dir(&directory).unwrap();
    fs::create_dir(&destination).unwrap();
    let service = midnight_platform::LocalFileSystem::new(None);
    for paths in [
        vec![file.clone(), directory.clone()],
        vec![file.clone(); 65],
    ] {
        let request = request(1, &paths, &destination);
        let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
        assert!(
            worker::Job::prepare_move(request, &service, Cancellation::default(), status).is_err()
        );
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        assert_eq!(fs::read(&file).unwrap(), b"original");
        assert!(directory.exists());
    }
}

#[test]
fn move_skip_or_cancel_blocks_every_source_capture() {
    for skip in [false, true] {
        let (_temp, parent) = protected_fixture();
        let paths = [parent.join("first"), parent.join("second")];
        let destination = parent.join("destination");
        fs::create_dir(&destination).unwrap();
        for path in &paths {
            fs::write(path, b"original").unwrap();
        }
        if skip {
            fs::write(destination.join("second"), b"existing").unwrap();
        }
        let cancel = Cancellation::default();
        let (mut job, status) = prepare_move(&paths, &destination, cancel.clone());
        for _ in 0..1000 {
            match job.step() {
                worker::Step::Conflict(conflict) => assert!(job.accept_decision(CopyDecision {
                    id: conflict.id,
                    version: conflict.version,
                    choice: CopyChoice::Skip
                })),
                worker::Step::Done => break,
                worker::Step::Continue => {
                    if !skip && snapshot(&status).phase == CopyOperationPhase::ValidatingBeforeMove
                    {
                        cancel.cancel();
                    }
                }
            }
        }
        let done = job.finish();
        assert_eq!(done.source_names_captured, 0);
        assert!(paths.iter().all(|path| path.exists()));
        assert_ne!(done.state, CopyOperationState::Completed);
    }
}

#[test]
fn move_cannot_replace_an_earlier_proof_from_the_same_batch() {
    let (_temp, parent) = protected_fixture();
    let (_other_temp, other_parent) = protected_fixture();
    let paths = [parent.join("same"), other_parent.join("same")];
    for path in &paths {
        fs::write(path, b"original").unwrap();
    }
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    let (mut job, _) = prepare_move(&paths, &destination, Cancellation::default());
    for _ in 0..1000 {
        if let worker::Step::Conflict(conflict) = job.step() {
            assert!(!conflict.can_replace);
            assert!(job.accept_decision(CopyDecision {
                id: conflict.id,
                version: conflict.version,
                choice: CopyChoice::Rename("other-name".into())
            }));
            break;
        }
    }
    drive(&mut job);
    assert_eq!(job.finish().source_names_captured, 2);
}

#[test]
fn move_cancel_after_first_capture_retains_that_original_and_leaves_remaining_names() {
    let (_temp, parent) = protected_fixture();
    let paths = [parent.join("first"), parent.join("second")];
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    for path in &paths {
        fs::write(path, b"original").unwrap();
    }
    let cancel = Cancellation::default();
    let (mut job, status) = prepare_move(&paths, &destination, cancel.clone());
    for _ in 0..1000 {
        assert!(matches!(job.step(), worker::Step::Continue));
        if snapshot(&status).source_names_captured == 1 {
            cancel.cancel();
            break;
        }
    }
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Cancelled);
    assert_eq!(done.source_names_captured, 1);
    assert!(!paths[0].exists());
    assert!(paths[1].exists());
    assert_eq!(
        fs::read(&done.source_recoveries[0].recovery.path).unwrap(),
        b"original"
    );
    assert_eq!(fs::read(destination.join("second")).unwrap(), b"original");
}

#[test]
fn move_accepts_64_pinned_files_without_retaining_64_byte_buffers() {
    let (_temp, parent) = protected_fixture();
    let paths: Vec<_> = (0..64)
        .map(|index| parent.join(format!("file-{index}")))
        .collect();
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    for path in &paths {
        fs::write(path, []).unwrap();
    }
    let (mut job, _) = prepare_move(&paths, &destination, Cancellation::default());
    for _ in 0..3000 {
        match job.step() {
            worker::Step::Done => break,
            worker::Step::Continue => {}
            worker::Step::Conflict(value) => panic!("unexpected conflict {value:?}"),
        }
    }
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.source_names_captured, 64);
    assert_eq!(done.source_recovery_count, 64);
    assert_eq!(done.source_recoveries.len(), 16);
    assert_eq!(done.source_directories, vec![parent]);
    assert!(paths.iter().all(|path| !path.exists()));
}

struct ReadFailure(Box<dyn TransferSource>);
impl TransferSource for ReadFailure {
    fn entry(&self) -> &TransferEntry {
        self.0.entry()
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        self.0.native_identity()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.revalidate(cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.0.rewind_for_validation(cancel)
    }
    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        Err(error(
            &self.entry().item.path,
            io::ErrorKind::Other,
            "Injected source read failure",
        ))
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.0.next_child(cancel)
    }
}
struct FailSecondSource(midnight_platform::LocalFileSystem);
impl TransferService for FailSecondSource {
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        let mut prepared = self.0.prepare_move_source(path, cancel)?;
        if path.file_name() == Some(OsStr::new("second")) {
            prepared.source = Box::new(ReadFailure(prepared.source));
        }
        Ok(prepared)
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.prepare_move_destination(path, cancel)
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.0.prepare_transfer_source(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.prepare_transfer_destination(path, cancel)
    }
}

#[test]
fn move_second_file_read_failure_blocks_capture_of_already_published_first_file() {
    let (_temp, parent) = protected_fixture();
    let paths = [parent.join("first"), parent.join("second")];
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    for path in &paths {
        fs::write(path, b"original").unwrap();
    }
    let request = request(1, &paths, &destination);
    let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
    let service = FailSecondSource(midnight_platform::LocalFileSystem::new(None));
    let mut job =
        worker::Job::prepare_move(request, &service, Cancellation::default(), status).unwrap();
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.files_completed, 1);
    assert_eq!(done.failed, 1);
    assert_eq!(done.source_names_captured, 0);
    assert!(paths.iter().all(|path| path.exists()));
    assert_eq!(fs::read(destination.join("first")).unwrap(), b"original");
    assert!(!destination.join("second").exists());
    assert_eq!(
        fs::read_dir(&destination).unwrap().count(),
        1,
        "all unpublished prepared stages must be cleaned"
    );
}

#[tokio::test]
async fn shared_clipboard_move_uses_the_same_verified_queue_and_recovery_policy() {
    let (_temp, parent) = protected_fixture();
    let source = parent.join("clipboard source 文档");
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&source, b"original").unwrap();
    let clipboard = Arc::new(FileClipboard {
        paths: vec![source.clone()],
        intent: FileClipboardIntent::Move,
    });
    let mut harness = Harness::native();
    harness
        .controller
        .enqueue_clipboard_move(ClipboardMoveRequest {
            id: OperationId(1),
            clipboard: clipboard.clone(),
            destination: destination.clone(),
        })
        .await;
    let done = harness.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.intent, TransferIntent::RecoverableMove);
    assert_eq!(done.source_names_captured, 1);
    assert!(!source.exists());
    assert_eq!(
        fs::read(&done.source_recoveries[0].recovery.path).unwrap(),
        b"original"
    );
    assert_eq!(
        fs::read(destination.join("clipboard source 文档")).unwrap(),
        b"original"
    );
    assert_eq!(clipboard.paths, vec![source]);
    harness.close().await;
}
