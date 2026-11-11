use super::*;
use std::{path::PathBuf, sync::atomic::AtomicUsize, thread::ThreadId};

#[derive(Debug)]
struct Sources {
    path: PathBuf,
    resolved: Arc<AtomicUsize>,
    dropped: Arc<Mutex<Option<ThreadId>>>,
}
impl CapturedChecksumFiles for Sources {
    fn stamp(&self) -> TreeViewStamp {
        TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            session: TreeSessionId(1),
            revision: TreeRevision(1),
        }
    }
    fn request(&self) -> TreeRequestId {
        TreeRequestId(1)
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<Vec<ChecksumTarget>, FsError> {
        self.resolved.fetch_add(1, Ordering::SeqCst);
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        Ok(vec![self.path.clone().into()])
    }
}
impl Drop for Sources {
    fn drop(&mut self) {
        *self.dropped.lock().unwrap() = Some(std::thread::current().id());
    }
}
struct Reading(Arc<Barrier>);
impl TransferService for Reading {
    fn prepare_transfer_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.0.entered.notify_one();
        let mut released = self.0.released.lock().unwrap();
        while !*released {
            released = self.0.wake.wait(released).unwrap();
        }
        self.0.returned.store(true, Ordering::Release);
        Err(invalid("Native call returned after cancellation."))
    }
    fn prepare_transfer_destination(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("calculation never writes")
    }
}
fn request(
    job: u64,
    dropped: Arc<Mutex<Option<ThreadId>>>,
    resolved: Arc<AtomicUsize>,
) -> CapturedChecksumRequest {
    CapturedChecksumRequest {
        job: ChecksumJobId(job),
        generation: ChecksumGeneration(1),
        sources: Arc::new(Sources {
            path: std::env::current_dir().unwrap().join("source"),
            resolved,
            dropped,
        }),
        algorithms: ChecksumAlgorithms::default(),
        recursive: false,
        limits: ChecksumLimits::default(),
    }
}

#[tokio::test]
async fn checksum_capture_provider_lives_through_blocked_read_and_queued_shutdown() {
    let _quota = checksum::test_quota_fixture().await;
    let barrier = Barrier::new();
    let _release = ReleaseOnDrop(barrier.clone());
    let (sender, _events) = mpsc::channel(32);
    let controller = Controller::new(
        Some(Arc::new(Reading(barrier.clone()))),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        Arc::new(Semaphore::new(1)),
    );
    let first_drop = Arc::new(Mutex::new(None));
    let second_drop = Arc::new(Mutex::new(None));
    let resolved = Arc::new(AtomicUsize::new(0));
    controller
        .start_captured(request(1, first_drop.clone(), resolved.clone()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), barrier.entered.notified())
        .await
        .unwrap();
    assert_eq!(
        resolved.load(Ordering::SeqCst),
        2,
        "provider rechecked after serial queue"
    );
    controller
        .start_captured(request(
            2,
            second_drop.clone(),
            Arc::new(AtomicUsize::new(0)),
        ))
        .await
        .unwrap();
    controller.release(ChecksumJobId(1), ChecksumGeneration(1));
    assert!(
        first_drop.lock().unwrap().is_none(),
        "cancellation cannot release active provider"
    );
    let shutdown = tokio::spawn(controller.shutdown());
    barrier.release();
    tokio::time::timeout(Duration::from_secs(5), shutdown)
        .await
        .unwrap()
        .unwrap();
    for dropped in [first_drop, second_drop] {
        let thread = dropped.lock().unwrap().expect("source retired");
        assert_ne!(
            thread,
            std::thread::current().id(),
            "executor released final provider"
        );
    }
    assert!(barrier.returned.load(Ordering::Acquire));
}

#[tokio::test]
async fn checksum_capture_rejected_admission_retires_provider_on_worker() {
    let _quota = checksum::test_quota_fixture().await;
    let (sender, _events) = mpsc::channel(1);
    let controller = Controller::new(
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        Arc::new(Semaphore::new(1)),
    );
    let dropped = Arc::new(Mutex::new(None));
    assert!(
        controller
            .start_captured(request(1, dropped.clone(), Arc::new(AtomicUsize::new(0))))
            .await
            .is_err()
    );
    assert_ne!(
        dropped.lock().unwrap().unwrap(),
        std::thread::current().id()
    );
    controller.shutdown().await;
}
