use super::*;

fn progress(state: CopyOperationState) -> CopyProgress {
    let mut progress =
        CopyProgress::queued(OperationId(1), "target".into(), 1, TransferIntent::Copy);
    progress.state = state;
    progress
}

fn event_state(event: Event) -> CopyOperationState {
    match event {
        Event::Copy(CopyEvent::Progress(progress)) => progress.state,
        _ => panic!("expected Copy progress"),
    }
}

#[tokio::test]
async fn a_blocked_initial_delivery_does_not_require_work_to_wait_or_overtake_it() {
    let (sender, mut receiver) = mpsc::channel(1);
    sender
        .send(Event::Copy(CopyEvent::Progress(Arc::new(progress(
            CopyOperationState::Preparing,
        )))))
        .await
        .unwrap();
    let outbox = CopyOutbox::pending(Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    });
    let (terminal, finished) = oneshot::channel();
    let delivery = tokio::spawn(
        outbox
            .clone()
            .deliver(progress(CopyOperationState::Queued), finished),
    );
    assert!(
        outbox
            .sender
            .try_send(Event::Copy(CopyEvent::Progress(Arc::new(progress(
                CopyOperationState::Running
            )))))
            .is_err()
    );
    // Computation may finish while the initial event still has no output capacity.
    terminal
        .send(progress(CopyOperationState::Completed))
        .unwrap();
    // Let the delivery task run (current-thread runtime): it must park on the full
    // outbox, neither finishing nor dropping the initial event.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(!delivery.is_finished());
    assert_eq!(
        event_state(receiver.recv().await.unwrap()),
        CopyOperationState::Preparing
    );
    assert_eq!(
        event_state(receiver.recv().await.unwrap()),
        CopyOperationState::Queued
    );
    assert_eq!(
        event_state(receiver.recv().await.unwrap()),
        CopyOperationState::Completed
    );
    delivery.await.unwrap();
}

#[tokio::test]
async fn ordered_delivery_allows_later_progress_after_the_initial_event() {
    let (sender, mut receiver) = mpsc::channel(2);
    let outbox = CopyOutbox::pending(Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    });
    let (terminal, finished) = oneshot::channel();
    let delivery = tokio::spawn(
        outbox
            .clone()
            .deliver(progress(CopyOperationState::Queued), finished),
    );
    assert_eq!(
        event_state(receiver.recv().await.unwrap()),
        CopyOperationState::Queued
    );
    outbox
        .send(Event::Copy(CopyEvent::Progress(Arc::new(progress(
            CopyOperationState::Running,
        )))))
        .await;
    assert_eq!(
        event_state(receiver.recv().await.unwrap()),
        CopyOperationState::Running
    );
    terminal
        .send(progress(CopyOperationState::Completed))
        .unwrap();
    assert_eq!(
        event_state(receiver.recv().await.unwrap()),
        CopyOperationState::Completed
    );
    delivery.await.unwrap();
}

#[tokio::test]
async fn a_closed_event_receiver_cannot_strand_native_job_teardown() {
    let (sender, receiver) = mpsc::channel(1);
    let outbox = CopyOutbox::pending(Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    });
    let (terminal, finished) = oneshot::channel();
    drop(receiver);
    terminal
        .send(progress(CopyOperationState::Cancelled))
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(1),
        outbox
            .clone()
            .deliver(progress(CopyOperationState::Queued), finished),
    )
    .await
    .unwrap();
    tokio::time::timeout(
        Duration::from_secs(1),
        outbox.send(Event::Copy(CopyEvent::Progress(Arc::new(progress(
            CopyOperationState::Running,
        ))))),
    )
    .await
    .unwrap();
}

/// Only `deliver` publishes a job's terminal state. The intermediate sender refuses
/// it from every other publisher, so each job reports its terminal state once.
#[tokio::test]
async fn the_intermediate_sender_refuses_terminal_progress() {
    let (sender, mut receiver) = mpsc::channel(8);
    let outbox = CopyOutbox::unblocked(Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    });
    for state in [
        CopyOperationState::Cancelled,
        CopyOperationState::Completed,
        CopyOperationState::CompletedWithErrors,
        CopyOperationState::Failed,
    ] {
        let terminal = || Event::Copy(CopyEvent::Progress(Arc::new(progress(state))));
        assert!(
            matches!(
                outbox.sender.try_send(terminal()),
                Err(mpsc::error::TrySendError::Closed(_))
            ),
            "{state:?}"
        );
        outbox.send(terminal()).await;
    }
    assert!(receiver.try_recv().is_err());
    outbox
        .sender
        .try_send(Event::Copy(CopyEvent::Progress(Arc::new(progress(
            CopyOperationState::Running,
        )))))
        .unwrap();
    assert_eq!(
        event_state(receiver.try_recv().unwrap()),
        CopyOperationState::Running
    );
}

#[tokio::test]
async fn heartbeat_retries_a_wait_clear_after_direct_status_fills_the_outbox() {
    let (sender, mut receiver) = mpsc::channel(1);
    let outbox = CopyOutbox::unblocked(Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    });
    let status = Arc::new(Mutex::new(progress(CopyOperationState::Preparing)));
    let tracker = Mutex::new(Tracker::default());
    status.lock().unwrap().wait_reason = Some(TransferWaitReason::Resources);
    publish(&outbox, snapshot(&status)).await;
    status.lock().unwrap().wait_reason = None;
    publish_progress_tick(&outbox, &status, sample_throughput(&status, &tracker));
    match receiver.recv().await.unwrap() {
        Event::Copy(CopyEvent::Progress(progress)) => {
            assert_eq!(progress.wait_reason, Some(TransferWaitReason::Resources))
        }
        _ => panic!("expected resource wait"),
    }
    publish_progress_tick(&outbox, &status, sample_throughput(&status, &tracker));
    match receiver.recv().await.unwrap() {
        Event::Copy(CopyEvent::Progress(progress)) => assert_eq!(progress.wait_reason, None),
        _ => panic!("expected retried wait clear"),
    }
}

#[tokio::test]
async fn stale_progress_is_refused_while_terminal_delivery_is_blocked() {
    let (sender, mut receiver) = mpsc::channel(1);
    let outbox = CopyOutbox::pending(Outbox {
        sender: sender.clone(),
        wakeup: Arc::new(|| {}),
    });
    let mut paused = progress(CopyOperationState::Running);
    paused.pause_requested = true;
    paused.paused = true;
    let stale = Event::Copy(CopyEvent::Progress(Arc::new(paused)));
    let (terminal, finished) = oneshot::channel();
    let delivery = tokio::spawn(
        outbox
            .clone()
            .deliver(progress(CopyOperationState::Queued), finished),
    );
    assert_eq!(
        event_state(receiver.recv().await.unwrap()),
        CopyOperationState::Queued
    );
    sender
        .send(Event::Copy(CopyEvent::Progress(Arc::new(progress(
            CopyOperationState::Preparing,
        )))))
        .await
        .unwrap();
    terminal
        .send(progress(CopyOperationState::Completed))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !outbox.sender.intermediates_closed() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !delivery.is_finished(),
        "terminal should still be blocked by the filled outbox"
    );
    assert!(matches!(
        outbox.sender.try_send(stale),
        Err(mpsc::error::TrySendError::Closed(_))
    ));
    assert_eq!(
        event_state(receiver.recv().await.unwrap()),
        CopyOperationState::Preparing
    );
    assert_eq!(
        event_state(receiver.recv().await.unwrap()),
        CopyOperationState::Completed
    );
    delivery.await.unwrap();
    assert!(receiver.try_recv().is_err());
}
