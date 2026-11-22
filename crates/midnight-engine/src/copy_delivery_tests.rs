use super::*;
use std::path::PathBuf;

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
    let Some(Event::Copy(CopyEvent::Progress(completed))) = receiver.recv().await else {
        panic!("expected terminal progress");
    };
    assert_eq!(completed.state, CopyOperationState::Completed);
    assert_eq!(completed.closure, None);
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

#[tokio::test]
async fn terminal_delivery_attaches_closure_totals_and_keeps_exact_recovery_details() {
    let (sender, mut receiver) = mpsc::channel(2);
    let outbox = CopyOutbox::pending(Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    });
    let (terminal, finished) = oneshot::channel();
    let delivery = tokio::spawn(outbox.deliver(progress(CopyOperationState::Queued), finished));
    let Some(Event::Copy(CopyEvent::Progress(initial))) = receiver.recv().await else {
        panic!("expected initial progress");
    };
    assert_eq!(initial.closure, None);

    let mut finished = progress(CopyOperationState::Cancelled);
    finished.intent = TransferIntent::Move;
    finished.requested_roots = 3;
    finished.completed_roots = 1;
    finished.files_completed = 2;
    finished.completed_bytes = 24;
    finished.skipped = 1;
    finished.failed = 3;
    let residual = PathBuf::from("target").join("recovery");
    finished.recovery_count = 1;
    finished.recovery_paths.push(residual.clone());
    finished.errors.push(FsError::from_io(
        "recover original",
        &residual,
        io::Error::other("keep this recovery object"),
    ));
    finished.discovery = OperationDiscovery::scanning(DiscoveryScope::Recursive);
    for index in 0..5 {
        finished.discovery.observe(&MutationItem {
            path: PathBuf::from(format!("file-{index}")),
            kind: EntryKind::File,
            metadata: EntryMetadata {
                size: Some(12),
                ..EntryMetadata::default()
            },
        });
    }
    finished.discovery.finish();

    terminal.send(finished).unwrap();
    let Some(Event::Copy(CopyEvent::Progress(finished))) = receiver.recv().await else {
        panic!("expected terminal progress");
    };
    assert_eq!(finished.state, CopyOperationState::Cancelled);
    assert_eq!(
        (finished.files_completed, finished.completed_bytes),
        (2, 24)
    );
    assert_eq!((finished.failed, finished.skipped), (3, 1));
    assert_eq!(finished.recovery_paths, [residual]);
    assert_eq!(
        finished.closure,
        Some(CopyClosureReceipt {
            unfinished_roots: 2,
            uncommitted_files: Some(3),
            uncommitted_bytes: Some(36),
            omitted_error_details: 2,
        })
    );
    delivery.await.unwrap();
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn terminal_closure_keeps_file_totals_unknown_for_root_only_discovery() {
    let (sender, mut receiver) = mpsc::channel(2);
    let outbox = CopyOutbox::pending(Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    });
    let (terminal, finished) = oneshot::channel();
    let delivery = tokio::spawn(outbox.deliver(progress(CopyOperationState::Queued), finished));
    assert_eq!(
        event_state(receiver.recv().await.unwrap()),
        CopyOperationState::Queued
    );

    let mut finished = progress(CopyOperationState::CompletedWithErrors);
    finished.discovery = OperationDiscovery::scanning(DiscoveryScope::SelectedRoots);
    finished.discovery.finish();
    finished.skipped = 1;
    terminal.send(finished).unwrap();
    let Some(Event::Copy(CopyEvent::Progress(finished))) = receiver.recv().await else {
        panic!("expected terminal progress");
    };
    assert_eq!(
        finished.closure,
        Some(CopyClosureReceipt {
            unfinished_roots: 1,
            uncommitted_files: None,
            uncommitted_bytes: None,
            omitted_error_details: 0,
        })
    );
    delivery.await.unwrap();
}

/// A job's root notices wait while the UI takes no events (WIN-8: it refreshed a panel
/// of 100,001 members after every 16): they reach it joined, at most
/// `MAX_ROOTS_PER_NOTICE` roots each, in order, and all before the terminal progress, so
/// the end of a cancelled selection is a few events away instead of thousands.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waiting_root_notices_reach_the_ui_joined_and_before_the_terminal_progress() {
    const UI_EVENTS: usize = 32;
    const ROOTS: u64 = 4_000;
    let (sender, mut receiver) = mpsc::channel(UI_EVENTS);
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
    for entry in 0..ROOTS {
        outbox
            .send(Event::Copy(CopyEvent::RootsCompleted {
                id: OperationId(1),
                sources: vec![CompletedCopySource::Archive(VirtualEntryId {
                    mount: 7,
                    entry,
                })],
            }))
            .await;
    }
    terminal
        .send(progress(CopyOperationState::Cancelled))
        .unwrap();
    let (mut events, mut roots) = (0, Vec::new());
    loop {
        match receiver.recv().await.unwrap() {
            Event::Copy(CopyEvent::RootsCompleted { sources, .. }) => {
                events += 1;
                assert!(sources.len() <= MAX_ROOTS_PER_NOTICE, "{}", sources.len());
                roots.extend(sources.into_iter().map(|source| match source {
                    CompletedCopySource::Archive(id) => id.entry,
                    _ => unreachable!(),
                }));
            }
            Event::Copy(CopyEvent::Progress(progress)) => {
                assert_eq!(progress.state, CopyOperationState::Cancelled);
                break;
            }
            _ => panic!("unexpected event"),
        }
    }
    assert_eq!(
        roots,
        (0..ROOTS).collect::<Vec<_>>(),
        "every root, in order"
    );
    let bound = UI_EVENTS + 2 * (ROOTS as usize).div_ceil(MAX_ROOTS_PER_NOTICE);
    assert!(
        events <= bound,
        "{events} events for {ROOTS} roots before the terminal progress; at most {bound}"
    );
    delivery.await.unwrap();
}
