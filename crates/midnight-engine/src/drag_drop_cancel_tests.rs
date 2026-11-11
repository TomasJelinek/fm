//! Late replies from blocked native destination validation must not undo cancellation.
use super::*;
use std::sync::{Condvar, atomic::AtomicBool};

#[derive(Default)]
struct BlockedDevice {
    started: AtomicBool,
    released: Mutex<bool>,
    wake: Condvar,
}
struct ReleaseDevice(Arc<BlockedDevice>);
impl Drop for ReleaseDevice {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = true;
        self.0.wake.notify_all();
    }
}
impl TransferService for BlockedDevice {
    fn native_drop_device(&self, _: &Path, _: &Cancellation) -> Result<Option<u64>, FsError> {
        panic!("cancelled drop cannot start device classification")
    }
    fn prepare_transfer_destination(
        &self,
        _: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.started.store(true, Ordering::SeqCst);
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
        if cancel.is_cancelled() {
            return Err(error(
                io::ErrorKind::Interrupted,
                "cancelled metadata lookup",
            ));
        }
        Err(error(
            io::ErrorKind::Unsupported,
            "test destination was not cancelled",
        ))
    }
    fn prepare_transfer_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        panic!("cancelled drop cannot start source preparation")
    }
}

#[tokio::test]
async fn cancelled_blocked_internal_drop_does_not_publish_a_late_failure() {
    let fixture = Harness::new();
    let service = Arc::new(BlockedDevice::default());
    let _release = ReleaseDevice(service.clone());
    let source = Arc::new(CapturedSource {
        files: fixture.request.files.clone(),
        dropped: Arc::default(),
    });
    let (sender, mut events) = mpsc::channel(128);
    let mut tasks = JoinSet::new();
    let mut controller = Controller::new(
        None,
        Some(service.clone()),
        fixture.sink.snapshots.clone(),
        fixture.sink.capacity.clone(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    controller
        .attach(
            fixture.sink.host.clone(),
            fixture.sink.targets.clone(),
            &mut tasks,
        )
        .await;
    controller
        .prepare_captured(DragId(1), PaneId::Right, NavigationId(5), source)
        .await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if matches!(
                events.recv().await,
                Some(Event::DragDrop(DragDropEvent::Prepared {
                    id: DragId(1),
                    ..
                }))
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();
    controller
        .internal_request(InternalDropRequest {
            id: DragId(1),
            destination: InternalDropDestination::PaneRoot {
                pane: PaneId::Right,
                navigation: NavigationId(5),
            },
            intent: InternalDropIntent::Auto,
            verification: TransferVerification::FullSha256,
        })
        .await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while !service.started.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    controller.cancel(DragId(1)).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if matches!(
                events.recv().await,
                Some(Event::DragDrop(DragDropEvent::Finished {
                    id: DragId(1),
                    outcome: DragFinish::Cancelled
                }))
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();
    *service.released.lock().unwrap() = true;
    service.wake.notify_all();
    let late = tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            if let Some(Event::DragDrop(
                event @ (DragDropEvent::Failed { .. } | DragDropEvent::DropAccepted { .. }),
            )) = events.recv().await
            {
                break event;
            }
        }
    })
    .await;
    assert!(
        late.is_err(),
        "cancelled drop published a stale result: {late:?}"
    );
    assert!(controller.pending.lock().unwrap().is_empty());
    controller.shutdown();
    while tasks.join_next().await.is_some() {}
}
