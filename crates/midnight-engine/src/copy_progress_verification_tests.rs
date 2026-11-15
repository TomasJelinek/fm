//! Observe verification through Controller events while real destination reads are held.
use super::*;
use std::{
    ffi::OsStr,
    io::{Read, Seek, SeekFrom, Write},
    sync::Condvar,
};

const READ_BLOCK: usize = 64 * 1024;

struct Gates {
    released: Mutex<[bool; 4]>,
    changed: Condvar,
    entered: AtomicUsize,
    entering: Notify,
    stages: AtomicUsize,
    cleaned: AtomicUsize,
}
impl Gates {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            // First file: read one block, then wait. Second file: wait before
            // any readback, then again after one block. No cross-job ordering.
            released: Mutex::new([true, false, false, false]),
            changed: Condvar::new(),
            entered: AtomicUsize::new(0),
            entering: Notify::new(),
            stages: AtomicUsize::new(0),
            cleaned: AtomicUsize::new(0),
        })
    }
    fn wait(&self, gate: usize) -> io::Result<()> {
        self.entered.fetch_or(1 << gate, Ordering::SeqCst);
        self.entering.notify_one();
        let (released, timeout) = self
            .changed
            .wait_timeout_while(
                self.released.lock().unwrap(),
                Duration::from_secs(10),
                |released| !released[gate],
            )
            .unwrap();
        if timeout.timed_out() && !released[gate] {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "verification fixture gate was not released",
            ));
        }
        Ok(())
    }
    fn release(&self, gate: usize) {
        self.released.lock().unwrap()[gate] = true;
        self.changed.notify_all();
    }
    async fn wait_entered(&self, gate: usize) {
        tokio::time::timeout(Duration::from_secs(4), async {
            while self.entered.load(Ordering::SeqCst) & (1 << gate) == 0 {
                self.entering.notified().await;
            }
        })
        .await
        .expect("destination read did not enter the expected gate");
    }
}
struct ReleaseGates(Arc<Gates>);
impl Drop for ReleaseGates {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = [true; 4];
        self.0.changed.notify_all();
    }
}

struct Service(Arc<Gates>);
impl TransferService for Service {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        midnight_platform::LocalFileSystem::new(None).prepare_transfer_source(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(Directory {
            inner: midnight_platform::LocalFileSystem::new(None)
                .prepare_transfer_destination(path, cancel)?,
            gates: self.0.clone(),
        }))
    }
}
struct Directory {
    inner: Arc<dyn TransferDirectory>,
    gates: Arc<Gates>,
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
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        let index = self.gates.stages.fetch_add(1, Ordering::SeqCst);
        assert!(index < 2, "fixture unexpectedly created another stage");
        Ok(Box::new(Stage {
            inner: self.inner.stage_file(cancel)?,
            gates: self.gates.clone(),
            index,
            reads: 0,
        }))
    }
}
struct Stage {
    inner: Box<dyn StagedTransferFile>,
    gates: Arc<Gates>,
    index: usize,
    reads: usize,
}
impl Read for Stage {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.reads < 2 {
            // Deliberately retain the real stage and the caller's I/O permit
            // across cancellation until this native-call fixture returns.
            self.gates.wait(self.index * 2 + self.reads)?;
        }
        let maximum = bytes.len().min(READ_BLOCK);
        let count = self.inner.read(&mut bytes[..maximum])?;
        self.reads += 1;
        Ok(count)
    }
}
impl Write for Stage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Seek for Stage {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        self.inner.seek(offset)
    }
}
impl StagedTransferFile for Stage {
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
        self.inner.cleanup()?;
        self.gates.cleaned.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
struct Observed {
    verified: u64,
    copied: u64,
}
impl Observed {
    async fn next(&mut self, harness: &mut Harness) -> Arc<CopyProgress> {
        loop {
            match harness.next().await {
                CopyEvent::Progress(progress) => {
                    assert_eq!(progress.id, OperationId(1));
                    assert!(progress.verified_bytes >= self.verified, "{progress:?}");
                    assert!(progress.copied_bytes >= self.copied, "{progress:?}");
                    self.verified = progress.verified_bytes;
                    self.copied = progress.copied_bytes;
                    return progress;
                }
                CopyEvent::RootCompleted { id, .. } => assert_eq!(id, OperationId(1)),
                event => panic!("unexpected copy decision: {event:?}"),
            }
        }
    }
    async fn verifying(
        &mut self,
        harness: &mut Harness,
        path: &Path,
        readback: u64,
    ) -> Arc<CopyProgress> {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let progress = self.next(harness).await;
                assert!(!progress.state.is_terminal(), "{progress:?}");
                if progress.phase == CopyOperationPhase::Verifying
                    && progress.current_path.as_deref() == Some(path)
                    && progress.current_file_verified == readback
                {
                    return progress;
                }
            }
        })
        .await
        .expect("missing partial verification event")
    }
    async fn terminal(&mut self, harness: &mut Harness) -> Arc<CopyProgress> {
        loop {
            let progress = self.next(harness).await;
            if progress.state.is_terminal() {
                return progress;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_sha256_events_report_partial_readback_and_reset_for_the_next_file() {
    two_files(TransferVerification::FullSha256).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sampled_events_report_sample_bytes_and_reset_for_the_next_file() {
    two_files(TransferVerification::SizeAndSamples).await;
}

async fn two_files(policy: TransferVerification) {
    let temp = fixture();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let destination = temp.path().join("destination");
    let first_bytes = vec![17; 600_003];
    let second_bytes = vec![93; 400_007];
    fs::write(&first, &first_bytes).unwrap();
    fs::write(&second, &second_bytes).unwrap();
    fs::create_dir(&destination).unwrap();
    let gates = Gates::new();
    let _release = ReleaseGates(gates.clone());
    let mut harness = Harness::new(Some(Arc::new(Service(gates.clone()))));
    let mut input = request(1, &[first.clone(), second.clone()], &destination);
    input.verification = policy;
    harness.controller.enqueue(input).await;
    let mut observed = Observed::default();
    let partial = observed
        .verifying(&mut harness, &first, READ_BLOCK as u64)
        .await;
    gates.wait_entered(1).await;
    assert_partial(&partial, policy, first_bytes.len() as u64);
    assert_eq!(partial.copied_bytes, first_bytes.len() as u64);
    assert_eq!(partial.verified_bytes, READ_BLOCK as u64);
    assert_eq!(partial.files_completed, 0);
    // A second actual timer event while the same read is blocked must preserve
    // write/readback counts; the witness is not a final-only status assertion.
    let held = observed
        .verifying(&mut harness, &first, READ_BLOCK as u64)
        .await;
    assert_eq!(held.copied_bytes, partial.copied_bytes);
    assert_eq!(held.verified_bytes, partial.verified_bytes);
    gates.release(1);

    let reset = observed.verifying(&mut harness, &second, 0).await;
    gates.wait_entered(2).await;
    let first_readback = policy.readback_bytes(first_bytes.len() as u64);
    let total_copied = (first_bytes.len() + second_bytes.len()) as u64;
    assert_eq!(reset.verification, policy);
    assert_eq!(reset.current_file_size, Some(second_bytes.len() as u64));
    assert_eq!(reset.current_file_copied, second_bytes.len() as u64);
    assert_eq!(reset.copied_bytes, total_copied);
    assert_eq!(reset.verified_bytes, first_readback);
    assert_eq!(reset.files_completed, 1);
    gates.release(2);

    let partial = observed
        .verifying(&mut harness, &second, READ_BLOCK as u64)
        .await;
    gates.wait_entered(3).await;
    assert_partial(&partial, policy, second_bytes.len() as u64);
    assert_eq!(partial.copied_bytes, total_copied);
    assert_eq!(partial.verified_bytes, first_readback + READ_BLOCK as u64);
    gates.release(3);
    let done = observed.terminal(&mut harness).await;
    harness.close().await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.verification, policy);
    assert_eq!(done.files_completed, 2);
    assert_eq!(done.completed_roots, 2);
    assert_eq!(done.failed, 0);
    assert_eq!(done.skipped, 0);
    assert_eq!(done.copied_bytes, total_copied);
    assert_eq!(done.completed_bytes, total_copied);
    let expected = match policy {
        TransferVerification::FullSha256 => total_copied,
        TransferVerification::SizeAndSamples => 2 * 3 * 65_536,
    };
    assert_eq!(done.verified_bytes, expected);
    assert_eq!(done.validation_bytes, 0);
    assert_eq!(done.current_file_size, None);
    assert_eq!(done.current_file_copied, 0);
    assert_eq!(done.current_file_verified, 0);
    assert_eq!(gates.stages.load(Ordering::SeqCst), 2);
    assert_eq!(fs::read(destination.join("first")).unwrap(), first_bytes);
    assert_eq!(fs::read(destination.join("second")).unwrap(), second_bytes);
}

fn assert_partial(progress: &CopyProgress, policy: TransferVerification, size: u64) {
    assert_eq!(progress.state, CopyOperationState::Running);
    assert_eq!(progress.verification, policy);
    assert_eq!(progress.current_file_size, Some(size));
    assert_eq!(progress.current_file_copied, size);
    assert!(progress.current_file_verified > 0);
    assert!(progress.current_file_verified < policy.readback_bytes(size));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_blocked_verification_retains_native_capacity_until_read_returns() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    let bytes = vec![41; 600_003];
    fs::write(&source, &bytes).unwrap();
    fs::create_dir(&destination).unwrap();
    let gates = Gates::new();
    let _release = ReleaseGates(gates.clone());
    let mut harness = Harness::new(Some(Arc::new(Service(gates.clone()))));
    harness
        .controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let mut observed = Observed::default();
    observed
        .verifying(&mut harness, &source, READ_BLOCK as u64)
        .await;
    gates.wait_entered(1).await;
    let held_capacity = (
        harness.io.available_permits(),
        harness.mutation.available_permits(),
    );
    assert_eq!(held_capacity, (0, 0));
    harness.controller.cancel(OperationId(1));
    let cancelled = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let progress = observed.next(&mut harness).await;
            assert!(!progress.state.is_terminal(), "{progress:?}");
            if progress.state == CopyOperationState::CancelRequested {
                break progress;
            }
        }
    })
    .await
    .expect("missing cancellation acknowledgement while read is held");
    assert_eq!(cancelled.current_file_verified, READ_BLOCK as u64);
    assert_eq!(cancelled.copied_bytes, bytes.len() as u64);
    assert_eq!(gates.cleaned.load(Ordering::SeqCst), 0);
    assert_eq!(
        (
            harness.io.available_permits(),
            harness.mutation.available_permits()
        ),
        held_capacity
    );
    gates.release(1);
    let done = observed.terminal(&mut harness).await;
    let returned_capacity = (
        harness.io.available_permits(),
        harness.mutation.available_permits(),
    );
    harness.close().await;
    assert_eq!(done.state, CopyOperationState::Cancelled, "{done:?}");
    assert_eq!(done.files_completed, 0);
    assert_eq!(done.copied_bytes, bytes.len() as u64);
    // The already-entered read finishes after cancellation, and its real bytes
    // remain accounted for before the next checkpoint stops the copy.
    assert_eq!(done.verified_bytes, 2 * READ_BLOCK as u64);
    assert_eq!(done.current_file_size, None);
    assert_eq!(done.current_file_verified, 0);
    assert_eq!(returned_capacity, (1, 1));
    assert!(gates.cleaned.load(Ordering::SeqCst) > 0);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    assert_eq!(fs::read(&source).unwrap(), bytes);
}
