use super::*;
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
fn move_removes_each_original_only_after_its_file_publishes_and_validates() {
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
            "sources must remain until the first file verification barrier"
        );
        assert!(matches!(job.step(), worker::Step::Continue));
        if snapshot(&status).phase == CopyOperationPhase::ValidatingBeforeMove {
            break;
        }
    }
    assert_eq!(snapshot(&status).files_completed, 1);
    assert_eq!(snapshot(&status).source_names_captured, 0);
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.intent, TransferIntent::RecoverableMove);
    assert_eq!(done.source_names_captured, 2);
    assert_eq!(done.source_recovery_count, 0);
    assert_eq!(done.source_files_removed, 2);
    assert_eq!(done.validation_bytes, 4_800_000);
    assert!(paths.iter().all(|path| !path.exists()));
    assert!(done.source_recoveries.is_empty());
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
fn move_skip_retains_that_file_and_cancel_keeps_unfinished_sources() {
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
        assert_eq!(done.source_files_removed, u64::from(skip));
        assert_eq!(paths[0].exists(), !skip);
        assert!(paths[1].exists());
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
    assert!(!destination.join("second").exists());
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
    assert_eq!(done.source_recovery_count, 0);
    assert_eq!(done.source_files_removed, 64);
    assert_eq!(done.source_recoveries.len(), 0);
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
fn move_second_file_read_failure_keeps_first_completed_move_and_second_original() {
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
    assert_eq!(done.source_files_removed, 1);
    assert!(!paths[0].exists());
    assert!(paths[1].exists());
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
    assert_eq!(done.source_files_removed, 1);
    assert!(done.source_recoveries.is_empty());
    assert_eq!(
        fs::read(destination.join("clipboard source 文档")).unwrap(),
        b"original"
    );
    assert_eq!(clipboard.paths, vec![source]);
    harness.close().await;
}

#[test]
fn move_removes_this_verified_source_before_copying_next_and_cancel_keeps_remainder() {
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
        if matches!(job.step(), worker::Step::Done) {
            break;
        }
        if snapshot(&status).source_files_removed == 1 {
            break;
        }
    }
    assert_eq!(
        snapshot(&status).source_files_removed,
        1,
        "source must actually finalize per file"
    );
    assert!(!paths[0].exists());
    assert_eq!(fs::read(destination.join("first")).unwrap(), b"original");
    assert!(
        !destination.join("second").exists(),
        "next file must wait for prior source removal"
    );
    assert_eq!(fs::read(&paths[1]).unwrap(), b"original");
    cancel.cancel();
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Cancelled);
    assert_eq!(done.source_files_removed, 1);
    assert_eq!(done.source_recovery_count, 0);
    assert_eq!(fs::read(&paths[1]).unwrap(), b"original");
}

#[test]
fn late_native_source_metadata_edit_after_capture_retains_original_and_stops_next_copy() {
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
        if snapshot(&status).source_names_captured == 1 {
            break;
        }
    }
    let recovery = snapshot(&status).source_recoveries[0].recovery.path.clone();
    let original_permissions = fs::metadata(&recovery).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&recovery, readonly).unwrap();
    drive(&mut job);
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::CompletedWithErrors);
    assert_eq!(done.source_files_removed, 0);
    assert_eq!(done.source_recovery_count, 1);
    assert_eq!(fs::read(&recovery).unwrap(), b"original");
    fs::set_permissions(recovery, original_permissions).unwrap();
    assert_eq!(fs::read(&paths[1]).unwrap(), b"original");
    assert!(!destination.join("second").exists());
}
