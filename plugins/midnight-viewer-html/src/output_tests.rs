use super::*;
use std::cell::RefCell;
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

thread_local! {
    /// Runs once on the reader's thread after a pass found every queue empty and
    /// before it decides whether to end, so a test can interleave the sender there.
    static QUEUES_EMPTY: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}
pub(super) fn pass_found_queues_empty() {
    if let Some(hook) = QUEUES_EMPTY.with_borrow_mut(Option::take) {
        hook();
    }
}

#[test]
fn closed_emitted_before_finish_is_written_when_it_races_the_empty_checks() {
    let (output, mut receiver) = channel();
    let sender = output.clone();
    // The browser closes after the reader found the queues empty but before the
    // reader looked at `finished`: Closed and finish() were both published by then.
    QUEUES_EMPTY.set(Some(Box::new(move || {
        sender.emit(header(1), api::EventKind::Closed);
        sender.finish();
    })));
    let event = receiver.next(&output);
    assert_eq!(
        event.map(|event| event.kind),
        Some(api::EventKind::Closed),
        "an event emitted before finish() must be written before the writer ends"
    );
    assert!(receiver.next(&output).is_none());
}

fn view(value: u8) -> (api::Header, api::Frame, ViewPaint) {
    (
        api::Header {
            session: 17,
            document: 23,
            sequence: 41,
        },
        api::Frame {
            viewport_revision: 31,
            acknowledged_input: 37,
            width: 2,
            height: 3,
            rgba: vec![value; 24],
        },
        ViewPaint {
            session: 17,
            document: 23,
            viewport_revision: 31,
            width: 2,
            height: 3,
            serial: 43,
        },
    )
}

#[test]
fn matching_pending_view_moves_its_original_allocation_after_patch() {
    let (output, mut receiver) = channel();
    let (header, frame, previous) = view(7);
    let pointer = frame.rgba.as_ptr();
    let capacity = frame.rgba.capacity();
    output.emit(header, api::EventKind::Ready);
    output.emit_view_frame(header, frame, previous.serial);
    let pixels = output
        .take_view_pixels(previous, |pixels| {
            assert_eq!(pixels, [7; 24]);
            pixels.fill(9);
            true
        })
        .unwrap();
    assert_eq!(pixels.as_ptr(), pointer);
    assert_eq!(pixels.capacity(), capacity);
    assert_eq!(pixels, [9; 24]);
    assert!(
        output
            .take_view_pixels(previous, |_| panic!("pixels already taken"))
            .is_none()
    );
    output.finish();
    assert_eq!(receiver.next(&output).unwrap().kind, api::EventKind::Ready);
    assert!(receiver.next(&output).is_none());
}

#[test]
fn mismatched_pending_view_identity_never_calls_patch_or_consumes_frame() {
    let (output, mut receiver) = channel();
    let (header, frame, previous) = view(7);
    let expected = frame.clone();
    output.emit_view_frame(header, frame, previous.serial);
    for wrong in [
        ViewPaint {
            session: previous.session + 1,
            ..previous
        },
        ViewPaint {
            document: previous.document + 1,
            ..previous
        },
        ViewPaint {
            viewport_revision: previous.viewport_revision + 1,
            ..previous
        },
        ViewPaint {
            width: previous.width + 1,
            ..previous
        },
        ViewPaint {
            height: previous.height + 1,
            ..previous
        },
        ViewPaint {
            serial: previous.serial + 1,
            ..previous
        },
    ] {
        assert!(
            output
                .take_view_pixels(wrong, |_| panic!("wrong view identity: {wrong:?}"))
                .is_none()
        );
    }
    let event = receiver.next(&output).unwrap();
    assert_eq!(event.header, header);
    assert_eq!(event.kind, api::EventKind::Frame(expected));
}

#[test]
fn delivered_view_cannot_be_reclaimed_from_output() {
    let (output, mut receiver) = channel();
    let (header, frame, previous) = view(7);
    output.emit_view_frame(header, frame, previous.serial);
    assert!(matches!(
        receiver.next(&output).unwrap().kind,
        api::EventKind::Frame(_)
    ));
    assert!(
        output
            .take_view_pixels(previous, |_| panic!("frame already delivered"))
            .is_none()
    );
}

#[test]
fn ordinary_or_popup_paint_replaces_view_tag_without_becoming_reusable() {
    let (output, mut receiver) = channel();
    let (header, frame, previous) = view(7);
    output.emit_view_frame(header, frame, previous.serial);
    let (_, ordinary, _) = view(9);
    let expected = ordinary.clone();
    output.emit(header, api::EventKind::Frame(ordinary));
    assert!(
        output
            .take_view_pixels(previous, |_| panic!("untagged paint"))
            .is_none()
    );
    let event = receiver.next(&output).unwrap();
    assert_eq!(event.header, header);
    assert_eq!(event.kind, api::EventKind::Frame(expected));
}

#[test]
fn rejected_view_patch_preserves_event_metadata_and_reusable_allocation() {
    for reclaim in [false, true] {
        let (output, mut receiver) = channel();
        let (header, frame, previous) = view(7);
        let expected = frame.clone();
        let pointer = frame.rgba.as_ptr();
        let capacity = frame.rgba.capacity();
        output.emit_view_frame(header, frame, previous.serial);
        let mut called = false;
        assert!(
            output
                .take_view_pixels(previous, |pixels| {
                    called = true;
                    assert_eq!(pixels, [7; 24]);
                    false
                })
                .is_none()
        );
        assert!(called);
        if reclaim {
            let pixels = output.take_view_pixels(previous, |_| true).unwrap();
            assert_eq!(pixels.as_ptr(), pointer);
            assert_eq!(pixels.capacity(), capacity);
            assert_eq!(pixels, expected.rgba);
        } else {
            let event = receiver.next(&output).unwrap();
            assert_eq!(event.header, header);
            assert_eq!(event.kind, api::EventKind::Frame(expected));
        }
    }
}

#[test]
fn newest_view_replaces_both_pixels_and_reclaim_identity() {
    let (output, _) = channel();
    let (header, frame, previous) = view(7);
    output.emit_view_frame(header, frame, previous.serial);
    let (_, replacement, _) = view(9);
    let newest = ViewPaint {
        serial: previous.serial + 1,
        ..previous
    };
    output.emit_view_frame(header, replacement, newest.serial);
    assert!(
        output
            .take_view_pixels(previous, |_| panic!("retired view paint"))
            .is_none()
    );
    assert_eq!(output.take_view_pixels(newest, |_| true).unwrap(), [9; 24]);
}

#[test]
fn retired_view_is_discarded_with_its_reclaim_identity() {
    let (output, mut receiver) = channel();
    let (header, frame, previous) = view(7);
    output.emit_view_frame(header, frame, previous.serial);
    output.emit(header, state("retired"));
    output.emit(header, api::EventKind::Ready);
    output.discard_presentation();
    assert!(
        output
            .take_view_pixels(previous, |_| panic!("discarded view paint"))
            .is_none()
    );
    output.finish();
    assert_eq!(receiver.next(&output).unwrap().kind, api::EventKind::Ready);
    assert!(receiver.next(&output).is_none());
}

#[test]
fn invalid_view_frame_keeps_prior_paint_and_queues_a_bounded_error() {
    let (output, mut receiver) = channel();
    let (header, frame, previous) = view(7);
    let expected = frame.clone();
    output.emit_view_frame(header, frame, previous.serial);
    let (_, mut invalid, _) = view(9);
    invalid.rgba.pop();
    output.emit_view_frame(header, invalid, previous.serial + 1);
    assert!(matches!(
        receiver.next(&output).unwrap().kind,
        api::EventKind::Error {
            code: api::ErrorCode::Runtime,
            ..
        }
    ));
    assert_eq!(
        output.take_view_pixels(previous, |_| true).unwrap(),
        expected.rgba
    );
    assert!(!output.failed.load(Ordering::Acquire));
}
