//! A refused incoming native drop is reported to the UI without blocking the
//! native callback, reserving copy capacity or revoking the native session.
use super::*;
use tokio::sync::watch;

#[test]
fn refused_native_drops_are_reported_without_waiting_or_reserving_capacity() {
    let Harness {
        sink, mut events, ..
    } = Harness::new();
    let sink = Arc::new(sink);
    sink.reject(error(
        io::ErrorKind::InvalidData,
        "The native file list exceeds the 4 MiB limit.",
    ));
    let Ok(DragDropEvent::Failed {
        id: None,
        error: reported,
    }) = events.try_recv()
    else {
        panic!("a refused drop must be reported without a gesture id");
    };
    assert_eq!(reported.kind, io::ErrorKind::InvalidData);
    assert_eq!(
        reported.message,
        "The native file list exceeds the 4 MiB limit."
    );
    assert_eq!(sink.capacity.capacity().available_permits(), 32);
    assert!(sink.pending.lock().unwrap().is_empty());

    // Under event backpressure the native callback still returns at once; the
    // report is dropped and the queued events stay untouched.
    let filler = || DragDropEvent::Hover {
        host_generation: 3,
        pane: None,
    };
    while sink.events.try_send(filler()).is_ok() {}
    let (returned, on_return) = std::sync::mpsc::channel();
    let callback = sink.clone();
    std::thread::spawn(move || {
        callback.reject(error(io::ErrorKind::WouldBlock, "The copy queue is full."));
        let _ = returned.send(());
    });
    on_return
        .recv_timeout(Duration::from_secs(5))
        .expect("a refusal report must never wait for event capacity");
    let mut queued = 0;
    while let Ok(event) = events.try_recv() {
        assert!(matches!(event, DragDropEvent::Hover { .. }));
        queued += 1;
    }
    assert_eq!(queued, 64);

    // A closed drop window has nobody to tell.
    sink.host.lifetime.cancel();
    sink.reject(error(
        io::ErrorKind::Interrupted,
        "The drop window has closed.",
    ));
    assert!(events.try_recv().is_err());
}

/// Raises one refused drop from inside its pump, as an OLE `Drop` callback does,
/// and publishes every later pump so the test can see the session stay open.
struct RefusesOneDrop(Arc<watch::Sender<u64>>);
struct RefusingSession {
    sink: Option<Arc<dyn FileDropSink>>,
    pumps: Arc<watch::Sender<u64>>,
}
impl FileDragDropService for RefusesOneDrop {
    fn open_session(
        &self,
        _: DragHost,
        _: SharedDropTargets,
        sink: Arc<dyn FileDropSink>,
    ) -> Result<Box<dyn FileDragDropSession>, FsError> {
        Ok(Box::new(RefusingSession {
            sink: Some(sink),
            pumps: self.0.clone(),
        }))
    }
}
impl FileDragDropSession for RefusingSession {
    fn service_pending(&mut self) -> Result<(), FsError> {
        if let Some(sink) = self.sink.take() {
            sink.reject(error(
                io::ErrorKind::InvalidData,
                "The file clipboard exceeds its 4096-item resource budget.",
            ));
        }
        self.pumps.send_modify(|pumps| *pumps += 1);
        Ok(())
    }
    fn start_drag(&mut self, _: &FileClipboard, _: &Cancellation) -> Result<DragFinish, FsError> {
        Ok(DragFinish::Unavailable)
    }
}

#[tokio::test]
async fn refused_native_drop_reaches_the_ui_and_keeps_the_native_session() {
    let fixture = Harness::new();
    let (pumps, mut pumped) = watch::channel(0u64);
    let (sender, mut events) = mpsc::channel(16);
    let mut tasks = JoinSet::new();
    let mut controller = Controller::new(
        crate::engine_lifetime::DragRetirement::testing(),
        Some(Arc::new(RefusesOneDrop(Arc::new(pumps)))),
        None,
        fixture.sink.snapshots.clone(),
        copy_operations::Admission::default(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let mut host = fixture.sink.host.clone();
    host.native = NativeDragWindow::Windows { hwnd: 1 };
    controller
        .attach(host, fixture.sink.targets.clone(), &mut tasks)
        .await;
    let reported = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(Event::DragDrop(DragDropEvent::Failed { id: None, error })) => break error,
                Some(Event::DragDrop(DragDropEvent::Ready {
                    supported: false, ..
                })) => panic!("a refused drop revoked native drag/drop"),
                None => panic!("drag event stream ended"),
                _ => {}
            }
        }
    })
    .await
    .expect("the refused drop must reach the UI");
    assert_eq!(reported.kind, io::ErrorKind::InvalidData);
    assert!(reported.message.contains("4096-item"));
    // A lost session is never pumped again; this one keeps running.
    let after = *pumped.borrow_and_update();
    let kept = tokio::time::timeout(Duration::from_secs(5), async {
        pumped
            .wait_for(|count| *count >= after + 2)
            .await
            .map(|_| ())
    })
    .await;
    controller.shutdown();
    while tasks.join_next().await.is_some() {}
    assert!(
        matches!(kept, Ok(Ok(()))),
        "the native session must stay open after a refused drop"
    );
}
