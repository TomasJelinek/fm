//! Request-scoped preparation protocol tests. No native plugin is needed here.
use super::*;
use crate::session::RequestState;

fn preparation() -> messages::PrepareSource {
    messages::PrepareSource {
        revision: 9,
        viewport: messages::Viewport {
            source_anchor: 48,
            backwards: true,
            max_bytes: 12,
            max_lines: 8,
        },
        max_scan_bytes: 32,
    }
}

fn shared(kind: u32) -> Arc<Shared> {
    let mut shared = crate::session::tests::shared();
    Arc::get_mut(&mut shared).unwrap().mode = viewer::MODE_SOURCE;
    let mut request = if kind == abi::REQUEST_PREPARE_SOURCE {
        RequestState::preparation(preparation())
    } else {
        RequestState::new(kind)
    };
    request.accepted = true;
    lock(&shared.state).requests.insert(41, request);
    shared
}

fn deliver(shared: &Shared, id: u64, kind: EventKind) {
    let event = abi::Event {
        request_id: id,
        session: 1,
        generation: 7,
        kind: 0,
        flags: 0,
        payload: loaded::empty(),
        content: loaded::empty(),
    };
    let bytes = kind.retained_bytes();
    process_event(shared, event, Ok(Some((kind, bytes))));
}

fn result() -> EventKind {
    EventKind::SourcePrepared(messages::SourcePrepared {
        revision: 9,
        frontier: 32,
        ready: true,
    })
}

#[test]
fn preparation_result_is_retained_then_completed_once() {
    let shared = shared(abi::REQUEST_PREPARE_SOURCE);
    deliver(&shared, 41, result());
    let state = lock(&shared.state);
    assert!(state.requests.contains_key(&41), "result is provisional");
    assert!(state.bytes >= size_of::<HostEvent>());
    drop(state);
    deliver(&shared, 41, EventKind::Completed);
    let state = lock(&shared.state);
    assert!(!state.requests.contains_key(&41));
    assert!(matches!(
        state.events[0].0.kind,
        EventKind::SourcePrepared(_)
    ));
    assert!(matches!(state.events[1].0.kind, EventKind::Completed));
    assert!(!shared.closing.load(Ordering::Acquire));
}

#[test]
fn missing_duplicate_wrong_kind_and_unsolicited_results_are_integrity_failures() {
    for scenario in 0..5 {
        let shared = shared(if scenario == 2 {
            abi::REQUEST_VIEWPORT
        } else {
            abi::REQUEST_PREPARE_SOURCE
        });
        match scenario {
            0 => deliver(&shared, 41, EventKind::Completed),
            1 => {
                deliver(&shared, 41, result());
                deliver(&shared, 41, result());
            }
            2 => deliver(&shared, 41, result()),
            3 => deliver(&shared, 42, result()),
            _ => {
                lock(&shared.state).requests.get_mut(&41).unwrap().accepted = false;
                deliver(&shared, 41, result());
            }
        }
        assert!(
            shared.closing.load(Ordering::Acquire),
            "scenario {scenario}"
        );
        assert!(matches!(
            lock(&shared.state).fatal.as_ref().unwrap().kind,
            EventKind::Failed(messages::Failure {
                code: abi::INVALID,
                ..
            })
        ));
    }
}

#[test]
fn preparation_rejects_bad_revision_frontier_and_unexpected_pages() {
    for kind in [
        EventKind::SourcePrepared(messages::SourcePrepared {
            revision: 8,
            frontier: 32,
            ready: true,
        }),
        EventKind::SourcePrepared(messages::SourcePrepared {
            revision: 9,
            frontier: 65,
            ready: false,
        }),
        EventKind::TextPosition(viewer::TextPosition {
            revision: 9,
            source_offset: 32,
            line: 1,
            column: 1,
        }),
    ] {
        let shared = shared(abi::REQUEST_PREPARE_SOURCE);
        deliver(&shared, 41, kind);
        assert!(shared.closing.load(Ordering::Acquire));
    }
}

#[test]
fn cancelled_and_failed_preparations_never_become_completed() {
    for terminal in [
        EventKind::Completed,
        EventKind::Failed(messages::Failure {
            code: abi::QUEUE_FULL,
            message: "temporary capacity".into(),
        }),
    ] {
        let shared = shared(abi::REQUEST_PREPARE_SOURCE);
        deliver(&shared, 41, result());
        lock(&shared.state).requests.get_mut(&41).unwrap().cancelled = true;
        deliver(&shared, 41, terminal);
        assert!(
            lock(&shared.state)
                .events
                .iter()
                .all(|(event, _)| { !matches!(event.kind, EventKind::Completed) })
        );
        assert!(!shared.closing.load(Ordering::Acquire));
    }
}

#[test]
fn native_optional_rejection_preserves_unsupported_and_temporary_statuses() {
    for code in [abi::UNSUPPORTED, abi::QUEUE_FULL, abi::PANICKED] {
        let shared = shared(abi::REQUEST_PREPARE_SOURCE);
        rejected(&shared, 41, code);
        let state = lock(&shared.state);
        assert!(matches!(
            &state.events[0].0.kind,
            EventKind::Failed(failure) if failure.code == code
        ));
        assert!(!shared.closing.load(Ordering::Acquire));
    }
}

#[test]
fn preparation_decoding_rejects_raw_content_and_charges_the_typed_result() {
    let payload = codec::encode(&messages::SourcePrepared {
        revision: 9,
        frontier: 32,
        ready: false,
    })
    .unwrap();
    let mut event = abi::Event {
        request_id: 41,
        session: 1,
        generation: 7,
        kind: abi::EVENT_SOURCE_PREPARED,
        flags: 0,
        payload: abi::Buffer {
            id: 1,
            bytes: loaded::slice(&payload),
        },
        content: loaded::empty(),
    };
    assert!(matches!(
        decode_event(event).unwrap(),
        Some((EventKind::SourcePrepared(_), bytes)) if bytes >= size_of::<HostEvent>()
    ));
    event.content = abi::Buffer {
        id: 2,
        bytes: loaded::slice(b"x"),
    };
    assert!(decode_event(event).is_err());
}
