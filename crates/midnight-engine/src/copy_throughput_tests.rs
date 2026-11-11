use super::*;
use std::ffi::OsStr;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn short_copy_and_move_events_retain_separate_transfer_and_readback_activity() {
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        let (_temp, parent) = moves::protected_fixture();
        let source = parent.join("speed-source");
        let destination = parent.join("speed-destination");
        fs::write(&source, b"measured bytes").unwrap();
        fs::create_dir(&destination).unwrap();
        let mut h = Harness::new(Some(Arc::new(moves::CrossDeviceService(
            midnight_platform::LocalFileSystem::new(None),
        ))));
        let request = request(1, std::slice::from_ref(&source), &destination);
        match intent {
            TransferIntent::Copy => h.controller.enqueue(request).await,
            TransferIntent::Move => h.controller.enqueue_move(request).await,
        }
        let done = h.terminal(1).await;
        h.close().await;
        assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
        assert_eq!(done.completed_roots, 1);
        assert!(done.current_file_size.is_none());
        assert_eq!(done.throughput.transfer_per_second, 0.0);
        assert_eq!(done.throughput.verification_per_second, 0.0);
        assert!(
            done.throughput
                .samples
                .iter()
                .any(|sample| sample.transfer_per_second > 0.0)
        );
        assert!(
            done.throughput
                .samples
                .iter()
                .any(|sample| sample.verification_per_second > 0.0)
        );
        assert!(
            done.throughput
                .samples
                .iter()
                .all(|sample| sample.items_per_second == 0.0)
        );
        if intent == TransferIntent::Move {
            assert!(done.validation_bytes > 0);
            assert!(!source.exists());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_copy_emits_zero_activity_without_releasing_native_capacity() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, vec![42; 600_000]).unwrap();
    fs::create_dir(&destination).unwrap();
    let entered = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let _release = ReleaseOnDrop(released.clone());
    let mut h = Harness::new(Some(Arc::new(BlockingService {
        inner: midnight_platform::LocalFileSystem::new(None),
        entered: entered.clone(),
        released: released.clone(),
        prepared: Arc::new(AtomicUsize::new(0)),
        read_before_block: true,
    })));
    h.controller
        .enqueue(request(1, &[source], &destination))
        .await;
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();
    let sampled = tokio::time::timeout(Duration::from_secs(2), async {
        let mut observed_activity = false;
        loop {
            if let CopyEvent::Progress(progress) = h.next().await {
                observed_activity |= progress.throughput.transfer_per_second > 0.0;
                if observed_activity && progress.throughput.transfer_per_second == 0.0 {
                    break progress;
                }
            }
        }
    })
    .await;
    let capacity = (h.io.available_permits(), h.mutation.available_permits());
    h.controller.cancel(OperationId(1));
    released.store(true, Ordering::Release);
    let done = h.terminal(1).await;
    h.close().await;
    let sampled = sampled.expect("blocked Copy must still emit measured zero activity");
    assert_eq!(sampled.state, CopyOperationState::Running);
    assert_eq!(sampled.throughput.transfer_per_second, 0.0);
    assert_eq!(sampled.throughput.verification_per_second, 0.0);
    assert!(sampled.copied_bytes > 0);
    assert_eq!(sampled.current_file_size, Some(600_000));
    assert_eq!(sampled.current_file_copied, sampled.copied_bytes);
    assert_eq!(sampled.completed_roots, 0);
    assert!(done.current_file_size.is_none());
    assert_eq!(done.current_file_copied, 0);
    assert_eq!(capacity, (0, 0));
    assert_eq!(done.state, CopyOperationState::Cancelled);
    assert_eq!(done.throughput.transfer_per_second, 0.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conflict_wait_and_cancellation_keep_history_with_zero_current_activity() {
    let temp = fixture();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let destination = temp.path().join("destination");
    fs::write(&first, b"first copied content").unwrap();
    fs::write(&second, b"second content").unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("second"), b"existing winner").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, &[first, second], &destination))
        .await;
    let waiting = loop {
        if let CopyEvent::Progress(progress) = h.next().await
            && progress.state == CopyOperationState::WaitingForDecision
        {
            break progress;
        }
    };
    h.conflict(1).await;
    h.controller.cancel(OperationId(1));
    let done = h.terminal(1).await;
    h.close().await;
    for progress in [waiting, done] {
        assert_eq!(progress.throughput.transfer_per_second, 0.0);
        assert_eq!(progress.throughput.verification_per_second, 0.0);
        assert!(
            progress
                .throughput
                .samples
                .iter()
                .any(|sample| sample.transfer_per_second > 0.0)
        );
    }
}

struct DelayedDirectoryDrop {
    inner: Arc<dyn TransferDirectory>,
    entered: Arc<Notify>,
    released: Arc<AtomicBool>,
}
impl Drop for DelayedDirectoryDrop {
    fn drop(&mut self) {
        self.entered.notify_one();
        while !self.released.load(Ordering::Acquire) {
            std::thread::park_timeout(Duration::from_millis(5));
        }
    }
}
impl TransferDirectory for DelayedDirectoryDrop {
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
        self.inner.stage_file(cancel)
    }
}
struct DelayedDropService {
    inner: midnight_platform::LocalFileSystem,
    entered: Arc<Notify>,
    released: Arc<AtomicBool>,
}
impl TransferService for DelayedDropService {
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
        Ok(Arc::new(DelayedDirectoryDrop {
            inner: self.inner.prepare_transfer_destination(path, cancel)?,
            entered: self.entered.clone(),
            released: self.released.clone(),
        }))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn throughput_timer_cannot_publish_terminal_before_native_resource_cleanup() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, b"completed content").unwrap();
    fs::create_dir(&destination).unwrap();
    let entered = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let _release = ReleaseOnDrop(released.clone());
    let mut h = Harness::new(Some(Arc::new(DelayedDropService {
        inner: midnight_platform::LocalFileSystem::new(None),
        entered: entered.clone(),
        released: released.clone(),
    })));
    h.controller
        .enqueue(request(1, &[source], &destination))
        .await;
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();
    let mut observed_idle_cleanup = false;
    let early = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let CopyEvent::Progress(progress) = h.next().await {
                if progress.state.is_terminal() {
                    break progress;
                }
                observed_idle_cleanup |= progress.state == CopyOperationState::Running
                    && !progress.throughput.samples.is_empty()
                    && progress.throughput.transfer_per_second == 0.0;
            }
        }
    })
    .await;
    let capacity = (h.io.available_permits(), h.mutation.available_permits());
    released.store(true, Ordering::Release);
    let premature = early.is_ok();
    let done = match early {
        Ok(progress) => progress,
        Err(_) => h.terminal(1).await,
    };
    h.close().await;
    assert_eq!(capacity, (0, 0));
    assert!(
        !premature,
        "terminal progress cannot precede native resource release"
    );
    assert!(
        observed_idle_cleanup,
        "blocked cleanup must still report zero current activity"
    );
    assert_eq!(done.state, CopyOperationState::Completed);
}

#[tokio::test]
async fn final_cleanup_preserves_an_already_requested_cancellation() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, b"completed content").unwrap();
    fs::create_dir(&destination).unwrap();
    let entered = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let _release = ReleaseOnDrop(released.clone());
    let service = DelayedDropService {
        inner: midnight_platform::LocalFileSystem::new(None),
        entered: entered.clone(),
        released: released.clone(),
    };
    let request = request(1, &[source], &destination);
    let status = Arc::new(Mutex::new(initial(&request.clone().into())));
    let cancel = Cancellation::default();
    let mut job = worker::Job::prepare(request, &service, cancel.clone(), status.clone()).unwrap();
    loop {
        match job.step() {
            worker::Step::Continue => {}
            worker::Step::Done => break,
            _ => panic!("fixture unexpectedly required a decision"),
        }
    }
    cancel.cancel();
    status.lock().unwrap().state = CopyOperationState::CancelRequested;
    let pending = tokio::task::spawn_blocking(move || job.finish());
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();
    let during_cleanup = snapshot(&status).state;
    released.store(true, Ordering::Release);
    let done = pending.await.unwrap();
    assert_eq!(during_cleanup, CopyOperationState::CancelRequested);
    assert_eq!(done.state, CopyOperationState::Cancelled);
}
