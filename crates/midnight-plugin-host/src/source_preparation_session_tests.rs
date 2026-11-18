use super::*;

fn preparation() -> messages::PrepareSource {
    messages::PrepareSource {
        revision: 9,
        viewport: messages::Viewport {
            source_anchor: 48,
            backwards: false,
            max_bytes: 12,
            max_lines: 8,
        },
        max_scan_bytes: 32,
    }
}

#[test]
fn preparation_requires_opened_source_revision_and_bounded_target() {
    let mut shared = tests::shared();
    let (commands, receiver) = mpsc::sync_channel(MAX_REQUESTS);
    let mut session = Session {
        shared: shared.clone(),
        commands,
        ids: AtomicU64::new(2),
    };
    assert!(session.prepare_source(preparation()).is_err());
    lock(&shared.state).opened = true;
    assert!(session.prepare_source(preparation()).is_err(), "text mode");
    drop(session);
    Arc::get_mut(&mut shared).unwrap().mode = crate::api::viewer::MODE_SOURCE;
    shared.closing.store(false, Ordering::Release);
    let (commands, receiver2) = mpsc::sync_channel(MAX_REQUESTS);
    session = Session {
        shared: shared.clone(),
        commands,
        ids: AtomicU64::new(2),
    };
    for request in [
        messages::PrepareSource {
            revision: 8,
            ..preparation()
        },
        messages::PrepareSource {
            viewport: messages::Viewport {
                source_anchor: 65,
                ..preparation().viewport
            },
            ..preparation()
        },
        messages::PrepareSource {
            max_scan_bytes: 0,
            ..preparation()
        },
    ] {
        assert!(session.prepare_source(request).is_err());
    }
    assert!(receiver.try_recv().is_err());
    assert!(receiver2.try_recv().is_err());
    assert!(lock(&shared.state).requests.is_empty());
}

#[test]
fn cancelled_and_closing_background_requests_keep_capacity_until_cleanup() {
    let busy = Arc::new(AtomicBool::new(false));
    let shared = tests::shared();
    let mut request = RequestState::preparation(preparation());
    request._background = Some(BackgroundPermit::reserve(busy.clone()).unwrap());
    lock(&shared.state).requests.insert(41, request);
    let (commands, _receiver) = mpsc::sync_channel(1);
    let session = Session {
        shared: shared.clone(),
        commands,
        ids: AtomicU64::new(42),
    };
    session.cancel(41);
    assert!(BackgroundPermit::reserve(busy.clone()).is_err());
    session.close();
    assert!(BackgroundPermit::reserve(busy.clone()).is_err());
    lock(&shared.state).requests.clear(); // Quiescent dispatcher cleanup.
    assert!(BackgroundPermit::reserve(busy).is_ok());
}

#[test]
fn fatal_protocol_error_keeps_background_permit_until_native_cleanup() {
    let busy = Arc::new(AtomicBool::new(false));
    let shared = tests::shared();
    let mut request = RequestState::preparation(preparation());
    request._background = Some(BackgroundPermit::reserve(busy.clone()).unwrap());
    lock(&shared.state).requests.insert(41, request);
    shared.fatal(41, "bad preparation result".into());
    assert!(BackgroundPermit::reserve(busy.clone()).is_err());
    lock(&shared.state).requests.clear();
    assert!(BackgroundPermit::reserve(busy).is_ok());
}

#[test]
fn foreground_command_capacity_is_available_while_background_is_reserved() {
    let busy = Arc::new(AtomicBool::new(false));
    let shared = tests::shared();
    let mut request = RequestState::preparation(preparation());
    request._background = Some(BackgroundPermit::reserve(busy).unwrap());
    lock(&shared.state).requests.insert(41, request);
    let (commands, receiver) = mpsc::sync_channel(MAX_REQUESTS);
    let session = Session {
        shared,
        commands,
        ids: AtomicU64::new(42),
    };
    assert!(session.viewport(preparation().viewport).is_ok());
    assert_eq!(receiver.try_recv().unwrap().kind, abi::REQUEST_VIEWPORT);
}

#[test]
fn cancellation_at_output_admission_converts_completed_and_drops_late_results() {
    let shared = tests::shared();
    let mut request = RequestState::preparation(preparation());
    request.cancelled = true;
    lock(&shared.state).requests.insert(41, request);
    shared.push(
        HostEvent {
            request_id: 41,
            generation: 7,
            kind: EventKind::SourcePrepared(messages::SourcePrepared {
                revision: 9,
                frontier: 32,
                ready: true,
            }),
        },
        64,
    );
    shared.push(
        HostEvent {
            request_id: 41,
            generation: 7,
            kind: EventKind::Completed,
        },
        0,
    );
    let state = lock(&shared.state);
    assert_eq!(state.events.len(), 1);
    assert!(matches!(state.events[0].0.kind, EventKind::Cancelled));
}

#[test]
fn cancellation_after_native_terminal_but_before_drain_revokes_queued_readiness() {
    let shared = tests::shared();
    lock(&shared.state)
        .requests
        .insert(41, RequestState::preparation(preparation()));
    shared.push(
        HostEvent {
            request_id: 41,
            generation: 7,
            kind: EventKind::SourcePrepared(messages::SourcePrepared {
                revision: 9,
                frontier: 32,
                ready: true,
            }),
        },
        64,
    );
    shared.push(
        HostEvent {
            request_id: 41,
            generation: 7,
            kind: EventKind::Completed,
        },
        0,
    );
    let (commands, _receiver) = mpsc::sync_channel(1);
    let session = Session {
        shared,
        commands,
        ids: AtomicU64::new(42),
    };
    session.cancel(41);
    assert!(matches!(
        session.drain(64).as_slice(),
        [HostEvent {
            kind: EventKind::Cancelled,
            ..
        }]
    ));
}

#[test]
fn locate_and_preparation_share_one_global_slot_while_foreground_can_be_admitted() {
    fn session() -> (Session, mpsc::Receiver<Command>) {
        let mut shared = tests::shared();
        Arc::get_mut(&mut shared).unwrap().mode = crate::api::viewer::MODE_SOURCE;
        lock(&shared.state).opened = true;
        let (commands, receiver) = mpsc::sync_channel(MAX_REQUESTS);
        (
            Session {
                shared,
                commands,
                ids: AtomicU64::new(2),
            },
            receiver,
        )
    }
    let (first, first_commands) = session();
    let (second, second_commands) = session();
    let prepare = first.prepare_source(preparation()).unwrap();
    assert_eq!(
        first_commands.try_recv().unwrap().kind,
        abi::REQUEST_PREPARE_SOURCE
    );
    assert!(
        second
            .locate(messages::Locate { source_anchor: 32 })
            .is_err()
    );
    assert!(second.viewport(preparation().viewport).is_ok());
    assert_eq!(
        second_commands.try_recv().unwrap().kind,
        abi::REQUEST_VIEWPORT
    );
    first.cancel(prepare);
    assert!(
        second
            .locate(messages::Locate { source_anchor: 32 })
            .is_err()
    );
    first.shared.push(
        HostEvent {
            request_id: prepare,
            generation: 7,
            kind: EventKind::Cancelled,
        },
        0,
    );
    let locate = second
        .locate(messages::Locate { source_anchor: 32 })
        .unwrap();
    assert_eq!(
        second_commands.try_recv().unwrap().kind,
        abi::REQUEST_LOCATE
    );
    assert!(first.prepare_source(preparation()).is_err());
    second.shared.push(
        HostEvent {
            request_id: locate,
            generation: 7,
            kind: EventKind::Completed,
        },
        0,
    );
    assert!(first.prepare_source(preparation()).is_ok());
}

#[test]
fn typed_admission_distinguishes_capacity_closing_and_invalid_without_parsing_messages() {
    let shared = tests::shared();
    let (commands, _receiver) = mpsc::sync_channel(1);
    let session = Session {
        shared: shared.clone(),
        commands,
        ids: AtomicU64::new(2),
    };
    assert!(matches!(
        session.try_prepare_source(preparation()),
        Err(AdmissionError::Invalid(_))
    ));
    assert!(session.try_viewport(preparation().viewport).is_ok());
    assert!(matches!(
        session.try_viewport(preparation().viewport),
        Err(AdmissionError::Capacity)
    ));
    session.close();
    assert!(matches!(
        session.try_viewport(preparation().viewport),
        Err(AdmissionError::Closing)
    ));
}
