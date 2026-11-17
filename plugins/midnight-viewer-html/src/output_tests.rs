use super::*;
use std::time::Duration;

fn header(sequence: u64) -> api::Header {
    api::Header {
        session: 1,
        document: 1,
        sequence,
    }
}
fn paint(value: u8) -> api::EventKind {
    api::EventKind::Frame(api::Frame {
        viewport_revision: 1,
        acknowledged_input: 0,
        width: 1,
        height: 1,
        rgba: vec![value; 4],
    })
}
fn state(title: &str) -> api::EventKind {
    api::EventKind::State(api::State {
        title: title.into(),
        url: String::new(),
        loading: false,
        editable: false,
        zoom: 1.0,
    })
}

#[test]
fn queued_paint_is_published_during_continuous_control_traffic() {
    let (output, mut receiver) = channel();
    output.emit(header(1), paint(7));
    for sequence in 2..=65 {
        output.emit(header(sequence), api::EventKind::Ready);
    }
    for sequence in 2..=9 {
        assert_eq!(receiver.next(&output).unwrap().header.sequence, sequence);
    }
    assert!(
        matches!(
            receiver.next(&output).unwrap().kind,
            api::EventKind::Frame(_)
        ),
        "a ready paint must not wait for control traffic to become idle"
    );
    assert_eq!(receiver.next(&output).unwrap().header.sequence, 10);
}

#[test]
fn coalesced_wake_tokens_do_not_lose_later_paints() {
    let (output, mut receiver) = channel();
    let (observed, complete) = mpsc::sync_channel(1);
    let (next, pending) = mpsc::sync_channel(1);
    let reader_output = output.clone();
    let reader = std::thread::spawn(move || {
        for _ in 0..100 {
            let event = receiver.next(&reader_output).unwrap();
            observed.send(event).unwrap();
            pending.recv().unwrap();
        }
    });
    for value in 1..=100 {
        output.emit(header(value.into()), paint(value));
        let event = complete.recv_timeout(Duration::from_secs(5)).unwrap();
        let api::EventKind::Frame(frame) = event.kind else {
            panic!("paint")
        };
        assert_eq!(frame.rgba, [value; 4]);
        next.send(()).unwrap();
    }
    reader.join().unwrap();
}

#[test]
fn frame_coalescing_and_control_overflow_keep_existing_bounds() {
    let (output, mut receiver) = channel();
    for value in 1..=100 {
        output.emit(header(value.into()), paint(value));
    }
    let api::EventKind::Frame(frame) = receiver.next(&output).unwrap().kind else {
        panic!("frame")
    };
    assert_eq!(frame.rgba, [100; 4]);
    for sequence in 0..CONTROL_CAPACITY as u64 {
        output.emit(header(101 + sequence), api::EventKind::Ready);
    }
    assert!(!output.failed.load(Ordering::Acquire));
    output.emit(header(1000), api::EventKind::Ready);
    assert!(output.failed.load(Ordering::Acquire));
}

#[test]
fn state_and_release_bursts_never_fail_the_transport() {
    let (output, mut receiver) = channel();
    // A title-changing script and a page releasing 2048 host assets while the
    // host is not reading used to overflow the 64-entry control queue.
    for title in 0..10_000 {
        output.emit(header(1), state(&title.to_string()));
    }
    for resource in 1..=2048 {
        output.emit(header(1), api::EventKind::ResourceRelease { resource });
    }
    for _ in 1..CONTROL_CAPACITY {
        output.emit(header(1), api::EventKind::Ready);
    }
    assert!(!output.failed.load(Ordering::Acquire));
    let mut controls = 0;
    let mut released = Vec::new();
    let mut states = Vec::new();
    output.finish();
    while let Some(event) = receiver.next(&output) {
        match event.kind {
            api::EventKind::Ready => controls += 1,
            api::EventKind::ResourceRelease { resource } => released.push(resource),
            api::EventKind::State(state) => states.push(state.title),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(controls, CONTROL_CAPACITY - 1);
    assert_eq!(released, (1..=2048).collect::<Vec<_>>());
    assert_eq!(states, ["9999"]);
}

#[test]
fn releases_wait_for_earlier_controls_of_the_same_resource() {
    let (output, mut receiver) = channel();
    output.emit(
        header(1),
        api::EventKind::ResourceRead {
            request: 3,
            resource: 9,
            offset: 0,
            length: 1,
        },
    );
    output.emit(header(1), api::EventKind::ResourceRelease { resource: 9 });
    assert!(matches!(
        receiver.next(&output).unwrap().kind,
        api::EventKind::ResourceRead { resource: 9, .. }
    ));
    assert_eq!(
        receiver.next(&output).unwrap().kind,
        api::EventKind::ResourceRelease { resource: 9 }
    );
}

#[test]
fn invalid_provider_events_become_bounded_errors_instead_of_failing_the_writer() {
    let (output, mut receiver) = channel();
    for kind in [
        api::EventKind::ResourceResolve {
            request: 1,
            base: 1,
            url: "a".repeat(70_000),
        },
        api::EventKind::FindResult(api::FindResult {
            request_sequence: 1,
            matches: 0,
            active: 1,
            complete: true,
        }),
    ] {
        output.emit(header(1), kind);
        let event = receiver.next(&output).unwrap();
        assert!(matches!(
            event.kind,
            api::EventKind::Error {
                code: api::ErrorCode::Runtime,
                ..
            }
        ));
        api::write_message(&mut Vec::new(), &api::Message::Event(event)).unwrap();
    }
    assert!(!output.failed.load(Ordering::Acquire));
}

#[test]
fn retired_presentation_is_discarded_while_controls_are_kept() {
    let (output, mut receiver) = channel();
    output.emit(header(1), paint(1));
    output.emit(header(1), state("retired"));
    output.emit(header(1), api::EventKind::Ready);
    output.discard_presentation();
    output.finish();
    assert_eq!(receiver.next(&output).unwrap().kind, api::EventKind::Ready);
    assert!(receiver.next(&output).is_none());
}

#[test]
fn finish_ends_the_writer_after_queued_events_without_a_closed_event() {
    let (output, mut receiver) = channel();
    let (done, finished) = mpsc::sync_channel(1);
    let reader_output = output.clone();
    let reader = std::thread::spawn(move || {
        let mut events = 0;
        while receiver.next(&reader_output).is_some() {
            events += 1;
        }
        done.send(events).unwrap();
    });
    output.emit(header(1), api::EventKind::Ready);
    output.finish();
    assert_eq!(finished.recv_timeout(Duration::from_secs(5)), Ok(1));
    reader.join().unwrap();
}
