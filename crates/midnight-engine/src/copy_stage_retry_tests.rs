//! Retain one owned pre-copy stage through validation retries and terminal cleanup.
use super::*;
use std::{
    ffi::OsStr,
    io::{Read, Seek, SeekFrom, Write},
};

#[derive(Default)]
struct Control {
    created: AtomicUsize,
    probes: AtomicUsize,
    fail_probe: AtomicBool,
    refuse_cleanup: AtomicBool,
    writes: AtomicUsize,
}
struct Service {
    native: moves::CrossDeviceService,
    control: Arc<Control>,
}
impl TransferService for Service {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.native.prepare_transfer_source(path, cancel)
    }
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        self.native.prepare_move_source(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(Directory {
            inner: self.native.prepare_transfer_destination(path, cancel)?,
            control: self.control.clone(),
        }))
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.prepare_transfer_destination(path, cancel)
    }
}
struct Directory {
    inner: Arc<dyn TransferDirectory>,
    control: Arc<Control>,
}
impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn identity(&self) -> TransferIdentity {
        self.inner.identity()
    }
    fn capabilities(&self) -> TransferCapabilities {
        self.inner.capabilities()
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
    fn prepare_target(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        self.inner.prepare_target(name, cancel)
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.reject_source_ancestor(source, cancel)
    }
    fn create_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.create_directory(name, cancel)
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        let index = self.control.created.fetch_add(1, Ordering::SeqCst);
        let path = self.path().join(format!(".fixture-owned-stage-{index}"));
        let file = fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        Ok(Box::new(Stage {
            file: Some(file),
            path,
            control: self.control.clone(),
        }))
    }
    fn stage_file_with_metadata(
        &self,
        _: TransferMetadataPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.stage_file(cancel)
    }
}
struct Stage {
    file: Option<fs::File>,
    path: PathBuf,
    control: Arc<Control>,
}
impl Read for Stage {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.file.as_mut().unwrap().read(bytes)
    }
}
impl Write for Stage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.control.writes.fetch_add(1, Ordering::SeqCst);
        self.file.as_mut().unwrap().write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().unwrap().flush()
    }
}
impl Seek for Stage {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.as_mut().unwrap().seek(position)
    }
}
struct Guard;
impl TransferReadGuard for Guard {
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
}
impl StagedTransferFile for Stage {
    fn modified_time(&self, _: &Cancellation) -> Result<std::time::SystemTime, FsError> {
        self.control.probes.fetch_add(1, Ordering::SeqCst);
        if self.control.fail_probe.load(Ordering::SeqCst) {
            return Err(FsError::from_io(
                "probe fixture modified time",
                &self.path,
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "temporary metadata probe refusal",
                ),
            ));
        }
        Ok(self
            .file
            .as_ref()
            .unwrap()
            .metadata()
            .unwrap()
            .modified()
            .unwrap())
    }
    fn sync(&mut self, _: &Cancellation) -> Result<(), FsError> {
        self.file
            .as_ref()
            .unwrap()
            .sync_all()
            .map_err(|error| FsError::from_io("sync fixture stage", &self.path, error))
    }
    fn len(&self) -> Result<u64, FsError> {
        Ok(self.file.as_ref().unwrap().metadata().unwrap().len())
    }
    fn rewind_for_validation(
        &mut self,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.control.probes.fetch_add(1, Ordering::SeqCst);
        if self.control.fail_probe.load(Ordering::SeqCst) {
            return Err(FsError::from_io(
                "validate fixture stage",
                &self.path,
                io::Error::new(io::ErrorKind::PermissionDenied, "temporary probe refusal"),
            ));
        }
        self.file.as_mut().unwrap().rewind().unwrap();
        Ok(Box::new(Guard))
    }
    fn publish_no_replace(
        &mut self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        panic!("test must stop before publication or source removal")
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        if self.file.is_none() {
            return Ok(());
        }
        if self.control.refuse_cleanup.load(Ordering::SeqCst) {
            return Err(FsError::from_io(
                "clean fixture stage",
                &self.path,
                io::Error::new(io::ErrorKind::PermissionDenied, "owned stage retained"),
            ));
        }
        drop(self.file.take());
        fs::remove_file(&self.path)
            .map_err(|error| FsError::from_io("clean fixture stage", &self.path, error))
    }
}
struct Fixture {
    job: Option<worker::Job>,
    control: Arc<Control>,
    cancel: Cancellation,
    source: PathBuf,
    destination: PathBuf,
    _temp: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        Self::with_sources(1)
    }
    fn with_sources(count: usize) -> Self {
        // Honor the QA-owned TEMP/TMPDIR: Linux native qualification needs its
        // selected ext4 fixture filesystem even when sources live on DrvFs.
        let temp = tempfile::Builder::new()
            .prefix("transfer-stage-")
            .tempdir()
            .unwrap();
        // The native recovery factory creates the same private parent used by
        // real Move staging. No fixture changes an existing directory's ACL.
        let native = midnight_platform::LocalFileSystem::new(None);
        let parent = native
            .prepare_transfer_destination(temp.path(), &Cancellation::default())
            .unwrap();
        fs::write(temp.path().join("seed"), b"old seed").unwrap();
        let target = parent
            .prepare_target(OsStr::new("seed"), &Cancellation::default())
            .unwrap()
            .unwrap();
        let mut seed = parent
            .stage_file_for_target(
                OsStr::new("seed"),
                target,
                TransferReplacementPolicy::RecoverableCurrentEntry,
                &Cancellation::default(),
            )
            .unwrap();
        seed.write_all(b"new seed").unwrap();
        let TransferPublication::Replaced { recovery, .. } = seed
            .publish_replace(OsStr::new("seed"), &Cancellation::default())
            .unwrap()
        else {
            panic!("private fixture parent was not created")
        };
        let parent = recovery.path.parent().unwrap().to_owned();
        drop(seed);
        let source = parent.join("source");
        let destination = parent.join("destination");
        let sources = (0..count)
            .map(|index| {
                let path = if index == 0 {
                    source.clone()
                } else {
                    parent.join(format!("source-{index}"))
                };
                fs::write(&path, b"original retained source").unwrap();
                path
            })
            .collect::<Vec<_>>();
        fs::create_dir(&destination).unwrap();
        let control = Arc::new(Control::default());
        control.fail_probe.store(true, Ordering::SeqCst);
        control.refuse_cleanup.store(true, Ordering::SeqCst);
        let service = Service {
            native: moves::CrossDeviceService(midnight_platform::LocalFileSystem::new(None)),
            control: control.clone(),
        };
        let cancel = Cancellation::default();
        let request = request(1, &sources, &destination);
        let status = Arc::new(Mutex::new(CopyProgress::queued(
            OperationId(1),
            destination.clone(),
            count as u64,
            TransferIntent::Move,
        )));
        let job = worker::Job::prepare_move(request, &service, cancel.clone(), status).unwrap();
        Self {
            job: Some(job),
            control,
            cancel,
            source,
            destination,
            _temp: temp,
        }
    }
    fn failure(&mut self) -> Arc<CopyFailure> {
        for _ in 0..40 {
            match self.job.as_mut().unwrap().step() {
                worker::Step::Continue => {}
                worker::Step::Failure(failure) => return failure,
                _ => panic!("expected stage validation failure before publication"),
            }
        }
        panic!("stage probe did not fail within bounded worker steps")
    }
    fn answer(&mut self, failure: &CopyFailure, choice: CopyChoice) {
        assert!(self.job.as_mut().unwrap().accept_decision(CopyDecision {
            id: failure.id,
            version: failure.version,
            choice,
        }));
    }
    fn finish(&mut self) -> CopyProgress {
        self.job.take().unwrap().finish()
    }
    fn first_stage(&self) -> PathBuf {
        self.destination.join(".fixture-owned-stage-0")
    }
}

#[test]
fn stage_probe_retries_reuse_one_owned_stage_before_any_content_write() {
    let mut fixture = Fixture::new();
    for _ in 0..8 {
        let failure = fixture.failure();
        assert!(failure.can_retry);
        fixture.answer(&failure, CopyChoice::Retry);
    }
    fixture.cancel.cancel();
    let done = fixture.finish();
    assert_eq!(
        fixture.control.created.load(Ordering::SeqCst),
        1,
        "retry must not abandon a stage and allocate another"
    );
    assert_eq!(fixture.control.writes.load(Ordering::SeqCst), 0);
    assert_eq!(done.state, CopyOperationState::Cancelled);
    assert!(
        done.errors
            .iter()
            .any(|error| error.path == fixture.first_stage())
    );
    assert_eq!(
        fs::read(&fixture.source).unwrap(),
        b"original retained source"
    );
    assert!(!fixture.destination.join("source").exists());
}

#[test]
fn stage_probe_retry_then_copy_cancel_keeps_original_cleanup_receipt() {
    let mut fixture = Fixture::new();
    let failure = fixture.failure();
    fixture.control.fail_probe.store(false, Ordering::SeqCst);
    fixture.answer(&failure, CopyChoice::Retry);
    for _ in 0..20 {
        assert!(matches!(
            fixture.job.as_mut().unwrap().step(),
            worker::Step::Continue
        ));
        if fixture.control.writes.load(Ordering::SeqCst) != 0 {
            break;
        }
    }
    assert!(fixture.control.writes.load(Ordering::SeqCst) > 0);
    fixture.cancel.cancel();
    let done = fixture.finish();
    assert!(
        done.errors
            .iter()
            .any(|error| error.path == fixture.first_stage()),
        "successful retry lost the original refused-cleanup stage receipt"
    );
    assert_eq!(fixture.control.created.load(Ordering::SeqCst), 1);
    assert_eq!(
        fs::read(&fixture.source).unwrap(),
        b"original retained source"
    );
    assert_eq!(fs::read_dir(&fixture.destination).unwrap().count(), 1);
}

#[test]
fn stage_probe_skip_reports_cleanup_refusal_without_removing_source() {
    let mut fixture = Fixture::new();
    let failure = fixture.failure();
    fixture.answer(&failure, CopyChoice::Skip);
    assert!(matches!(
        fixture.job.as_mut().unwrap().step(),
        worker::Step::Continue
    ));
    let done = fixture.finish();
    assert_eq!(done.skipped, 1);
    assert!(
        done.errors
            .iter()
            .any(|error| error.path == fixture.first_stage()
                && error.message.contains("owned stage retained"))
    );
    assert_eq!(done.source_files_removed, 0);
    assert_eq!(
        fs::read(&fixture.source).unwrap(),
        b"original retained source"
    );
    assert!(fixture.first_stage().exists());
}

#[test]
fn stage_probe_cancel_cleans_retained_stage_after_refusal_clears() {
    let mut fixture = Fixture::new();
    fixture.failure();
    fixture
        .control
        .refuse_cleanup
        .store(false, Ordering::SeqCst);
    fixture.cancel.cancel();
    let done = fixture.finish();
    assert_eq!(done.state, CopyOperationState::Cancelled);
    assert_eq!(
        fs::read_dir(&fixture.destination).unwrap().count(),
        0,
        "cancel must still own the failed-probe stage when cleanup becomes available"
    );
    assert_eq!(
        fs::read(&fixture.source).unwrap(),
        b"original retained source"
    );
}

#[tokio::test]
async fn metadata_probe_cleanup_refusal_stops_preparation_with_exact_residual() {
    let mut fixture = Fixture::new();
    drop(fixture.job.take());
    let mut harness = Harness::new(Some(Arc::new(Service {
        native: moves::CrossDeviceService(midnight_platform::LocalFileSystem::new(None)),
        control: fixture.control.clone(),
    })));
    harness
        .controller
        .enqueue_metadata(MetadataTransferRequest {
            input: MetadataTransferInput::Paths(request(
                1,
                std::slice::from_ref(&fixture.source),
                &fixture.destination,
            )),
            intent: TransferIntent::Copy,
            metadata: TransferMetadataPolicy::RequireModifiedTime,
        })
        .await;
    let mut offered_retry = false;
    let done = loop {
        match harness.next().await {
            CopyEvent::Failure(failure) => {
                offered_retry |= failure.can_retry;
                harness
                    .controller
                    .decide(CopyDecision {
                        id: failure.id,
                        version: failure.version,
                        choice: CopyChoice::Cancel,
                    })
                    .await;
            }
            CopyEvent::Progress(progress) if progress.state.is_terminal() => break progress,
            _ => {}
        }
    };
    harness.close().await;
    assert!(
        !offered_retry,
        "unrecoverable owned-stage cleanup must not permit a fresh stage"
    );
    assert_eq!(done.state, CopyOperationState::Failed);
    assert_eq!(
        done.failed, 1,
        "the same preflight residual is recorded once"
    );
    assert_eq!(done.errors.len(), 1);
    assert!(
        done.errors
            .iter()
            .any(|error| error.path == fixture.first_stage()
                && error.message.contains("owned stage retained"))
    );
    assert!(done.message.contains("temporary metadata probe refusal"));
    assert_eq!(fixture.control.created.load(Ordering::SeqCst), 1);
    assert!(fixture.first_stage().exists());
    assert_eq!(
        fs::read(&fixture.source).unwrap(),
        b"original retained source"
    );
    assert!(!fixture.destination.join("source").exists());
}

#[tokio::test]
async fn metadata_probe_failure_with_successful_cleanup_still_offers_retry() {
    let mut fixture = Fixture::new();
    drop(fixture.job.take());
    fixture
        .control
        .refuse_cleanup
        .store(false, Ordering::SeqCst);
    let mut harness = Harness::new(Some(Arc::new(Service {
        native: moves::CrossDeviceService(midnight_platform::LocalFileSystem::new(None)),
        control: fixture.control.clone(),
    })));
    harness
        .controller
        .enqueue_metadata(MetadataTransferRequest {
            input: MetadataTransferInput::Paths(request(
                1,
                std::slice::from_ref(&fixture.source),
                &fixture.destination,
            )),
            intent: TransferIntent::Copy,
            metadata: TransferMetadataPolicy::RequireModifiedTime,
        })
        .await;
    let failure = loop {
        match harness.next().await {
            CopyEvent::Failure(failure) => break failure,
            CopyEvent::Progress(progress) => assert!(!progress.state.is_terminal()),
            _ => {}
        }
    };
    let can_retry = failure.can_retry;
    harness
        .controller
        .decide(CopyDecision {
            id: failure.id,
            version: failure.version,
            choice: CopyChoice::Cancel,
        })
        .await;
    let done = harness.terminal(1).await;
    harness.close().await;
    assert!(
        can_retry,
        "ordinary transient preparation failures remain retryable"
    );
    assert_eq!(done.state, CopyOperationState::Cancelled);
    assert_eq!(fs::read_dir(&fixture.destination).unwrap().count(), 0);
    assert_eq!(
        fs::read(&fixture.source).unwrap(),
        b"original retained source"
    );
}

#[test]
fn stage_probe_skip_keeps_cleanup_receipt_after_full_error_history() {
    let mut fixture = Fixture::with_sources(18);
    fixture
        .control
        .refuse_cleanup
        .store(false, Ordering::SeqCst);
    for _ in 0..17 {
        let failure = fixture.failure();
        fixture.answer(&failure, CopyChoice::Skip);
        assert!(matches!(
            fixture.job.as_mut().unwrap().step(),
            worker::Step::Continue
        ));
    }
    let failure = fixture.failure();
    fixture.control.refuse_cleanup.store(true, Ordering::SeqCst);
    fixture.answer(&failure, CopyChoice::Skip);
    assert!(matches!(
        fixture.job.as_mut().unwrap().step(),
        worker::Step::Continue
    ));
    let done = fixture.finish();
    let residual = fixture.destination.join(".fixture-owned-stage-17");
    assert_eq!(done.errors.len(), 16);
    assert!(
        done.errors
            .iter()
            .any(|error| error.path == residual && error.message.contains("owned stage retained")),
        "bounded error history discarded the actionable abandoned-stage receipt"
    );
    assert_eq!(done.source_files_removed, 0);
    assert!(residual.exists());
    assert_eq!(
        fs::read(&fixture.source).unwrap(),
        b"original retained source"
    );
    assert_eq!(
        fs::read(fixture.source.with_file_name("source-17")).unwrap(),
        b"original retained source"
    );
    assert_eq!(fixture.control.writes.load(Ordering::SeqCst), 0);
}
