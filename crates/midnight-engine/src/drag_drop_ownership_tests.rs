//! Accepted typed drops retain provider authority until queue admission or rejection.
use super::*;

#[path = "drag_drop_rejection_tests.rs"]
mod rejection;

#[path = "drag_drop_cancel_tests.rs"]
mod cancellation;

#[derive(Debug)]
struct CapturedSource {
    files: FileClipboard,
    dropped: Arc<Mutex<Option<std::thread::ThreadId>>>,
}
impl CapturedClipboardFiles for CapturedSource {
    fn intent(&self) -> FileClipboardIntent {
        FileClipboardIntent::Copy
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<FileClipboard, FsError> {
        if cancel.is_cancelled() {
            return Err(error(io::ErrorKind::Interrupted, "capture cancelled"));
        }
        Ok(self.files.clone())
    }
}
impl Drop for CapturedSource {
    fn drop(&mut self) {
        *self.dropped.lock().unwrap() = Some(std::thread::current().id());
    }
}

#[tokio::test]
async fn accepted_tree_drag_retains_provider_until_rejection_and_retires_on_worker() {
    let fixture = Harness::new();
    let owner = std::thread::current().id();
    let dropped = Arc::new(Mutex::new(None));
    let source = Arc::new(CapturedSource {
        files: fixture.request.files.clone(),
        dropped: dropped.clone(),
    });
    let weak = Arc::downgrade(&source);
    let (sender, mut events) = mpsc::channel(128);
    let mut tasks = JoinSet::new();
    let mut controller = Controller::new(
        crate::engine_lifetime::DragRetirement::testing(),
        None,
        None,
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
        .prepare_captured(DragId(1), PaneId::Right, NavigationId(5), source.clone())
        .await;
    drop(source);
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
            intent: InternalDropIntent::Copy,
            verification: TransferVerification::SizeAndSamples,
        })
        .await;
    let token = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Event::DragDrop(DragDropEvent::DropAccepted {
                token,
                intent,
                verification,
                ..
            })) = events.recv().await
            {
                assert_eq!(intent, TransferIntent::Copy);
                assert_eq!(verification, Some(TransferVerification::SizeAndSamples));
                break token;
            }
        }
    })
    .await
    .unwrap();
    // Wait for the source worker to retire its old gesture owner. The accepted
    // request must independently retain the provider while the UI decides.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        weak.upgrade().is_some(),
        "accepted drop lost its provider before admission"
    );
    controller.reject(token);
    tokio::time::timeout(Duration::from_secs(3), async {
        while dropped.lock().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(weak.upgrade().is_none());
    assert_ne!(dropped.lock().unwrap().unwrap(), owner);
    assert_eq!(fixture.sink.capacity.capacity().available_permits(), 32);
    controller.shutdown();
    while tasks.join_next().await.is_some() {}
}

#[test]
fn unattached_drag_backlog_retires_after_runtime_shutdown() {
    struct Notice(std::sync::mpsc::Sender<std::thread::ThreadId>);
    impl Drop for Notice {
        fn drop(&mut self) {
            let _ = self.0.send(std::thread::current().id());
        }
    }
    let (notice, retired) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut tasks = JoinSet::new();
    let controller = {
        let _entered = runtime.enter();
        let (sender, _events) = mpsc::channel(1);
        Controller::new(
            crate::engine_lifetime::DragRetirement::testing(),
            None,
            None,
            crate::test_snapshots(Default::default()),
            copy_operations::Admission::default(),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        )
    };
    controller
        .state
        .lock()
        .unwrap()
        .retired
        .push(Box::new(Notice(notice)));
    drop(runtime);
    drop(tasks);
    drop(controller);
    assert_ne!(
        retired.recv_timeout(Duration::from_secs(2)).unwrap(),
        std::thread::current().id()
    );
}
