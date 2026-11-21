//! A refused incoming native drop is reported to the UI without blocking the
//! native callback, reserving copy capacity or revoking the native session.
use super::*;
use tokio::sync::watch;

fn filler() -> DragDropEvent {
    DragDropEvent::Hover {
        host_generation: 99,
        pane: None,
    }
}

fn refused_message(sink: &Sink) -> Option<String> {
    sink.refused
        .lock()
        .as_ref()
        .map(|(_, error)| error.message.clone())
}

#[test]
fn refused_native_drops_keep_the_latest_report_without_waiting_or_reserving_capacity() {
    let Harness {
        sink, mut events, ..
    } = Harness::new();
    let sink = Arc::new(sink);
    sink.reject(error(
        io::ErrorKind::InvalidData,
        "The native file list exceeds the 4 MiB limit.",
    ));
    {
        let refused = sink.refused.lock();
        let (lifetime, reported) = refused.as_ref().expect("a refused drop must be kept");
        assert_eq!(reported.kind, io::ErrorKind::InvalidData);
        assert!(!lifetime.is_cancelled());
    }
    // The report takes no event slot, copy permit or pending admission.
    assert!(events.try_recv().is_err());
    assert_eq!(sink.capacity.capacity().available_permits(), 32);
    assert!(sink.pending.lock().unwrap().is_empty());

    // With the event queue full, the native callback still returns at once, the
    // newer refusal replaces the undelivered one and the queued events stay.
    while sink.events.try_send(filler()).is_ok() {}
    let (returned, on_return) = std::sync::mpsc::channel();
    let callback = sink.clone();
    std::thread::spawn(move || {
        callback.reject(error(
            io::ErrorKind::WouldBlock,
            "The application cannot receive a drop right now.",
        ));
        let _ = returned.send(());
    });
    on_return
        .recv_timeout(Duration::from_secs(5))
        .expect("a refusal report must never wait for event capacity");
    assert_eq!(
        refused_message(&sink).as_deref(),
        Some("The application cannot receive a drop right now.")
    );
    let mut queued = 0;
    while let Ok(event) = events.try_recv() {
        assert!(matches!(event, DragDropEvent::Hover { .. }));
        queued += 1;
    }
    assert_eq!(queued, 64);

    // A closed drop window has nobody to tell.
    sink.refused.lock().take();
    sink.host.lifetime.cancel();
    sink.reject(error(
        io::ErrorKind::Interrupted,
        "The drop window has closed.",
    ));
    assert_eq!(refused_message(&sink), None);
}

/// Runs `forward` for the harness sink with the UI outbox full. Returns once
/// `forward` holds a drag event it cannot deliver: nothing more reaches the UI,
/// and no refusal is taken from its slot, until the test drains the outbox.
async fn stalled_forward(
    sink: &Sink,
    events: mpsc::Receiver<DragDropEvent>,
    tasks: &mut JoinSet<()>,
) -> (mpsc::Receiver<Event>, Cancellation) {
    let (sender, ui) = mpsc::channel(1);
    sender.try_send(Event::DragDrop(filler())).unwrap();
    let cancel = Cancellation::default();
    tasks.spawn(forward(
        events,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        sink.hover.clone(),
        sink.refused.clone(),
        cancel.clone(),
    ));
    sink.events.try_send(filler()).unwrap();
    // All 64 event slots are free again only after `forward` took that event.
    drop(sink.events.reserve_many(64).await.unwrap());
    (ui, cancel)
}

/// Stops `forward` as `Controller::shutdown` does: cancel it, then end its wait.
async fn stop_forward(sink: &Sink, cancel: Cancellation, tasks: &mut JoinSet<()>) {
    cancel.cancel();
    sink.hover.changed.notify_one();
    while tasks.join_next().await.is_some() {}
}

/// Drains the UI outbox until the Left hover sentinel arrives and returns the drag
/// events delivered before it. `forward` reports a pending refusal before a hover, so
/// a repeated or late refusal arrives before the sentinel.
async fn events_before_left_hover(ui: &mut mpsc::Receiver<Event>) -> Vec<DragDropEvent> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut before = Vec::new();
        loop {
            match ui.recv().await.expect("drag event stream ended") {
                Event::DragDrop(DragDropEvent::Hover {
                    pane: Some(PaneId::Left),
                    ..
                }) => return before,
                Event::DragDrop(event) => before.push(event),
                _ => {}
            }
        }
    })
    .await
    .expect("the hover sentinel must reach the UI")
}

fn refusals(events: &[DragDropEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, DragDropEvent::Failed { id: None, .. }))
        .count()
}

#[tokio::test]
async fn refusal_raised_while_the_ui_is_stalled_is_reported_once_after_it_drains() {
    let Harness {
        sink,
        events,
        request,
    } = Harness::new();
    let mut tasks = JoinSet::new();
    let (mut ui, cancel) = stalled_forward(&sink, events, &mut tasks).await;
    // A full drag event queue refuses the drop, and that refusal is still reported.
    while sink.events.try_send(filler()).is_ok() {}
    let refusal = sink.try_accept(request).unwrap_err();
    assert_eq!(refusal.kind, io::ErrorKind::WouldBlock);
    sink.reject(refusal);
    let reported = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::DragDrop(DragDropEvent::Failed { id: None, error }) =
                ui.recv().await.expect("drag event stream ended")
            {
                break error;
            }
        }
    })
    .await
    .expect("the refusal must reach the UI once it drains");
    assert_eq!(
        reported.message,
        "The application cannot receive a drop right now."
    );
    sink.hover(Some(PaneId::Left));
    assert_eq!(
        refusals(&events_before_left_hover(&mut ui).await),
        0,
        "a refusal is reported once"
    );
    stop_forward(&sink, cancel, &mut tasks).await;
}

#[tokio::test]
async fn refusal_for_a_window_that_closed_before_delivery_is_dropped() {
    let Harness { sink, events, .. } = Harness::new();
    let mut tasks = JoinSet::new();
    let (mut ui, cancel) = stalled_forward(&sink, events, &mut tasks).await;
    sink.reject(error(
        io::ErrorKind::InvalidInput,
        "The drop destination changed.",
    ));
    sink.host.lifetime.cancel();
    sink.hover(Some(PaneId::Left));
    assert_eq!(
        refusals(&events_before_left_hover(&mut ui).await),
        0,
        "a closed window's refusal must not reach the UI"
    );
    stop_forward(&sink, cancel, &mut tasks).await;
}

/// The next drag event that reaches the UI.
async fn reported(ui: &mut mpsc::Receiver<Event>) -> DragDropEvent {
    let Ok(Some(Event::DragDrop(event))) =
        tokio::time::timeout(Duration::from_secs(5), ui.recv()).await
    else {
        panic!("no drag report reached the UI");
    };
    event
}

/// With the clock paused, time advances only while every task waits for a timer.
/// `forward` has none, so each report reaches the UI at the instant it was made.
#[tokio::test(start_paused = true)]
async fn reports_reach_the_ui_without_waiting_for_a_timer() {
    let Harness { sink, events, .. } = Harness::new();
    let (sender, mut ui) = mpsc::channel(4);
    let cancel = Cancellation::default();
    let mut tasks = JoinSet::new();
    tasks.spawn(forward(
        events,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        sink.hover.clone(),
        sink.refused.clone(),
        cancel.clone(),
    ));
    let made = tokio::time::Instant::now();
    sink.hover(Some(PaneId::Left));
    assert!(matches!(
        reported(&mut ui).await,
        DragDropEvent::Hover {
            pane: Some(PaneId::Left),
            ..
        }
    ));
    // A refusal precedes the hover that changed with it.
    sink.reject(error(
        io::ErrorKind::InvalidData,
        "The native file list exceeds the 4 MiB limit.",
    ));
    sink.hover(Some(PaneId::Right));
    assert!(matches!(
        reported(&mut ui).await,
        DragDropEvent::Failed { id: None, .. }
    ));
    assert!(matches!(
        reported(&mut ui).await,
        DragDropEvent::Hover {
            pane: Some(PaneId::Right),
            ..
        }
    ));
    assert_eq!(
        tokio::time::Instant::now(),
        made,
        "a report waited for a timer"
    );
    // An unchanged hover is not delivered again.
    sink.hover(Some(PaneId::Right));
    assert!(
        tokio::time::timeout(Duration::from_secs(1), ui.recv())
            .await
            .is_err(),
        "an unchanged hover reached the UI again"
    );
    stop_forward(&sink, cancel, &mut tasks).await;
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
                "The dropped file list is malformed or not Unicode.",
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
    let mut resident_tasks = ResidentTasks::default();
    let (_resident_owner, mut controller) = Controller::testing(
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
        .attach(host, fixture.sink.targets.clone(), &mut resident_tasks)
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
    assert!(reported.message.contains("malformed or not Unicode"));
    // A lost session is never pumped again; this one keeps running.
    let after = *pumped.borrow_and_update();
    let kept = tokio::time::timeout(Duration::from_secs(5), async {
        pumped
            .wait_for(|count| *count >= after + 2)
            .await
            .map(|_| ())
    })
    .await;
    // Nothing published after the refusal revoked readiness or repeated it.
    let sink = controller
        .state
        .lock()
        .unwrap()
        .host
        .as_ref()
        .unwrap()
        .sink
        .clone();
    sink.hover(Some(PaneId::Left));
    let later = events_before_left_hover(&mut events).await;
    controller.shutdown();
    while tasks.join_next().await.is_some() {}
    while resident_tasks.join_next().await.is_some() {}
    assert!(
        matches!(kept, Ok(Ok(()))),
        "the native session must stay open after a refused drop"
    );
    assert!(
        !later.iter().any(|event| matches!(
            event,
            DragDropEvent::Ready {
                supported: false,
                ..
            } | DragDropEvent::Failed { .. }
        )),
        "later events: {later:?}"
    );
}
