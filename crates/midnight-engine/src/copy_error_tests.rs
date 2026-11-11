use super::*;
use std::{
    ffi::OsStr,
    io::{Read, Seek, SeekFrom, Write},
};

async fn failure(h: &mut Harness, id: u64) -> Arc<CopyFailure> {
    loop {
        match h.next().await {
            CopyEvent::Failure(failure) if failure.id == OperationId(id) => return failure,
            CopyEvent::Progress(progress) if progress.id == OperationId(id) => {
                assert!(
                    !progress.state.is_terminal(),
                    "expected failure decision, got {progress:?}"
                );
            }
            _ => {}
        }
    }
}

async fn decide(h: &Harness, error: &CopyFailure, choice: CopyChoice) {
    h.controller
        .decide(CopyDecision {
            id: error.id,
            version: error.version,
            choice,
        })
        .await;
}

#[tokio::test]
async fn unavailable_destination_waits_for_user_instead_of_finishing_the_job() {
    let temp = fixture();
    let source = temp.path().join("source.bin");
    let destination = temp.path().join("not-ready");
    fs::write(&source, b"retained source").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, &[source], &destination))
        .await;
    loop {
        if let CopyEvent::Progress(progress) = h.next().await {
            assert!(
                !progress.state.is_terminal(),
                "failed operation needs a user decision"
            );
            if progress.state == CopyOperationState::WaitingForDecision {
                break;
            }
        }
    }
    h.controller.cancel(OperationId(1));
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Cancelled);
    h.close().await;
}

#[tokio::test]
async fn destination_retry_repeats_failure_then_works_without_stale_consent() {
    let temp = fixture();
    let source = temp.path().join("source.bin");
    let destination = temp.path().join("not-ready");
    fs::write(&source, b"retained source").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let first = failure(&mut h, 1).await;
    assert!(first.can_retry);
    assert!(!first.can_skip);
    assert_eq!(h.io.available_permits(), 1);
    decide(&h, &first, CopyChoice::Retry).await;
    let second = failure(&mut h, 1).await;
    assert!(second.version > first.version);
    fs::create_dir(&destination).unwrap();
    // Force a name conflict after successful re-preparation; error Retry/Skip All
    // must never be accepted as overwrite consent with a reused version.
    fs::write(destination.join("source.bin"), b"keep destination").unwrap();
    decide(&h, &second, CopyChoice::Retry).await;
    let conflict = h.conflict(1).await;
    assert!(conflict.version > second.version);
    decide(&h, &first, CopyChoice::SkipAllErrors).await;
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::Rename("copied.bin".into()),
        })
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed);
    assert_eq!(done.failed, 0);
    assert!(done.errors.is_empty());
    assert_eq!(
        fs::read(destination.join("source.bin")).unwrap(),
        b"keep destination"
    );
    assert_eq!(
        fs::read(destination.join("copied.bin")).unwrap(),
        fs::read(source).unwrap()
    );
    h.close().await;
}

#[tokio::test]
async fn partial_disk_full_write_pauses_and_retries_without_corrupting_or_recopying() {
    let temp = fixture();
    let source = temp.path().join("source.bin");
    let destination = temp.path().join("target");
    fs::create_dir(&destination).unwrap();
    let bytes: Vec<_> = (0..700_123).map(|i| (i % 251) as u8).collect();
    fs::write(&source, &bytes).unwrap();
    let mut h = Harness::new(Some(Arc::new(DiskFullService {
        inner: midnight_platform::LocalFileSystem::new(None),
        fail: Arc::new(AtomicBool::new(true)),
        repeat: false,
    })));
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let error = failure(&mut h, 1).await;
    assert_eq!(error.stage, CopyErrorStage::Writing);
    assert_eq!(error.error.kind, io::ErrorKind::StorageFull);
    assert!(error.can_retry && error.can_skip);
    assert!(!destination.join("source.bin").exists());
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert_eq!(h.io.available_permits(), 1);
    decide(&h, &error, CopyChoice::Retry).await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed);
    assert_eq!(done.files_completed, 1);
    assert_eq!(done.copied_bytes, bytes.len() as u64);
    assert_eq!(done.completed_bytes, bytes.len() as u64);
    assert_eq!(done.verified_bytes, bytes.len() as u64);
    assert_eq!(done.failed, 0);
    assert_eq!(fs::read(destination.join("source.bin")).unwrap(), bytes);
    assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
    h.close().await;
}

#[tokio::test]
async fn cancel_partial_write_cleans_stage_and_releases_the_next_queued_job() {
    let temp = fixture();
    let destination = temp.path().join("target");
    fs::create_dir(&destination).unwrap();
    let first = temp.path().join("first.bin");
    let next = temp.path().join("next.bin");
    fs::write(&first, b"never publish this partially written file").unwrap();
    fs::write(&next, b"next queued transfer").unwrap();
    let mut h = Harness::new(Some(Arc::new(DiskFullService {
        inner: midnight_platform::LocalFileSystem::new(None),
        fail: Arc::new(AtomicBool::new(true)),
        repeat: false,
    })));
    h.controller
        .enqueue(request(1, std::slice::from_ref(&first), &destination))
        .await;
    let error = failure(&mut h, 1).await;
    h.controller
        .enqueue(request(2, std::slice::from_ref(&next), &destination))
        .await;
    assert_eq!(h.io.available_permits(), 1);
    decide(&h, &error, CopyChoice::Cancel).await;
    let cancelled = h.terminal(1).await;
    assert_eq!(cancelled.state, CopyOperationState::Cancelled);
    assert!(!cancelled.errors.is_empty());
    let completed = h.terminal(2).await;
    assert_eq!(completed.state, CopyOperationState::Completed);
    assert!(!destination.join("first.bin").exists());
    assert_eq!(
        fs::read(&first).unwrap(),
        b"never publish this partially written file"
    );
    assert_eq!(
        fs::read(destination.join("next.bin")).unwrap(),
        fs::read(next).unwrap()
    );
    assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
    h.close().await;
}

#[tokio::test]
async fn skip_all_errors_is_job_scoped_and_never_grants_overwrite_consent() {
    let temp = fixture();
    let destination = temp.path().join("target");
    fs::create_dir(&destination).unwrap();
    let sources: Vec<_> = ["first.bin", "second.bin", "existing.bin"]
        .into_iter()
        .map(|name| {
            let path = temp.path().join(name);
            fs::write(&path, b"retain source and all of its original bytes").unwrap();
            path
        })
        .collect();
    fs::write(destination.join("existing.bin"), b"retain destination").unwrap();
    let mut h = Harness::new(Some(Arc::new(DiskFullService {
        inner: midnight_platform::LocalFileSystem::new(None),
        fail: Arc::new(AtomicBool::new(true)),
        repeat: true,
    })));
    h.controller
        .enqueue(request(1, &sources, &destination))
        .await;
    let first = failure(&mut h, 1).await;
    decide(&h, &first, CopyChoice::SkipAllErrors).await;
    // A second matching write error is skipped, but the subsequent existing
    // destination still requires its own ordinary overwrite decision.
    let conflict = loop {
        match h.next().await {
            CopyEvent::Conflict(conflict) => break conflict,
            CopyEvent::Failure(error) => panic!("same error should be skipped: {error:?}"),
            CopyEvent::Progress(progress) => assert!(!progress.state.is_terminal()),
            _ => {}
        }
    };
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::Skip,
        })
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Failed);
    assert_eq!(done.failed, 2);
    assert_eq!(done.skipped, 3);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
    assert_eq!(
        fs::read(destination.join("existing.bin")).unwrap(),
        b"retain destination"
    );
    for source in &sources {
        assert_eq!(
            fs::read(source).unwrap(),
            b"retain source and all of its original bytes"
        );
    }
    h.controller
        .enqueue(request(2, &sources[..1], &destination))
        .await;
    let next = failure(&mut h, 2).await;
    assert_eq!(next.stage, CopyErrorStage::Writing);
    decide(&h, &next, CopyChoice::Cancel).await;
    assert_eq!(h.terminal(2).await.state, CopyOperationState::Cancelled);
    h.close().await;
}

struct DiskFullService {
    inner: midnight_platform::LocalFileSystem,
    fail: Arc<AtomicBool>,
    repeat: bool,
}
impl TransferService for DiskFullService {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.inner.prepare_transfer_source(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(DiskFullDirectory {
            inner: self.inner.prepare_transfer_destination(path, cancel)?,
            fail: self.fail.clone(),
            repeat: self.repeat,
        }))
    }
}
struct DiskFullDirectory {
    inner: Arc<dyn TransferDirectory>,
    fail: Arc<AtomicBool>,
    repeat: bool,
}
impl TransferDirectory for DiskFullDirectory {
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn identity(&self) -> TransferIdentity {
        self.inner.identity()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn inspect(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.inner.inspect(name, cancel)
    }
    fn reject_source_ancestor(
        &self,
        identity: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.reject_source_ancestor(identity, cancel)
    }
    fn create_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.create_directory(name, cancel)
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        Ok(Box::new(DiskFullStage {
            inner: self.inner.stage_file(cancel)?,
            fail: self.fail.clone(),
            partial: false,
            repeat: self.repeat,
        }))
    }
}
struct DiskFullStage {
    inner: Box<dyn StagedTransferFile>,
    fail: Arc<AtomicBool>,
    partial: bool,
    repeat: bool,
}
impl Read for DiskFullStage {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.inner.read(bytes)
    }
}
impl Seek for DiskFullStage {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        self.inner.seek(offset)
    }
}
impl Write for DiskFullStage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.partial {
            self.partial = false;
            self.fail.store(self.repeat, Ordering::Release);
            return Err(io::ErrorKind::StorageFull.into());
        }
        if self.fail.load(Ordering::Acquire) && !bytes.is_empty() {
            self.partial = true;
            return self.inner.write(&bytes[..bytes.len().min(17)]);
        }
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl StagedTransferFile for DiskFullStage {
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.sync(cancel)
    }
    fn len(&self) -> Result<u64, FsError> {
        self.inner.len()
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        self.inner.publish_no_replace(name, cancel)
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        self.inner.cleanup()
    }
}
