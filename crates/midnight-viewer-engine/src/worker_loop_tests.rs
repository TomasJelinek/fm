//! Event-driven loop, input batching and window-level inputs.
use super::*;
use midnight_plugin_host::api::abi::{INVALID, QUEUE_FULL, UNSUPPORTED};

fn complete_page(worker: &mut Worker, output: &Mailbox, rows: usize) {
    let source: String = (0..rows)
        .map(|row| format!("line {row}: {}\n", "word ".repeat(row % 7 + 1)))
        .collect();
    let mut decoded = decoded_page(&source);
    decoded.source.offset = 0;
    decoded.has_before = false;
    decoded.has_after = false;
    decoded.anchors = source
        .char_indices()
        .map(|(at, _)| viewer::Anchor {
            text_offset: at as u32,
            source_offset: at as u64,
        })
        .chain(std::iter::once(viewer::Anchor {
            text_offset: source.len() as u32,
            source_offset: source.len() as u64,
        }))
        .collect();
    worker.length = source.len() as u64;
    assert!(worker.set_page(Page::text(decoded, &worker.appearance), output));
}

fn frame(worker: &mut Worker, output: &Mailbox) -> ViewerFrame {
    worker.paint(output).unwrap();
    output
        .take()
        .into_iter()
        .find_map(|event| match event {
            ViewerOutput::Frame(frame) => Some(*frame),
            _ => None,
        })
        .expect("frame")
}

#[test]
fn idle_worker_blocks_and_retries_only_work_without_a_wake_signal() {
    let (mut worker, _) = worker();
    assert_eq!(worker.retry_delay(), None, "an idle viewer does not poll");
    worker.pending_open = true;
    assert_eq!(
        worker.retry_delay(),
        None,
        "file identities, sessions and settings wake the worker"
    );
    worker.dirty = true;
    assert_eq!(
        worker.retry_delay(),
        Some(RETRY),
        "a deferred paint is retried"
    );
    worker.dirty = false;
    worker.web_backlog = true;
    assert_eq!(worker.retry_delay(), Some(Duration::ZERO));
    worker.web_backlog = false;
    // An image actor finishing its teardown wakes the worker when it ends, as a closing
    // plugin session does when its dispatcher ends.
    let (release, teardown) = std::sync::mpsc::channel::<()>();
    let (_, events) = std::sync::mpsc::sync_channel(1);
    worker.closing_image = Some(crate::image_worker::ImageWorker::for_test(
        events,
        Some(Box::new(move || {
            let _ = teardown.recv();
        })),
    ));
    assert_eq!(worker.retry_delay(), None, "a closing actor is not polled");
    release.send(()).unwrap();
    worker.closing_image = None;
    assert_eq!(worker.retry_delay(), None);
}

#[test]
fn a_pending_open_resumes_on_the_exit_wake_of_a_closing_image_actor() {
    let (mut worker, output) = worker();
    captured_order(&mut worker, &["first.png", "second.md"]);
    let (sender, receiver) = std::sync::mpsc::sync_channel(4);
    let wakes = Arc::new(crate::input_queue::Wakes::new(sender));
    let wake: Arc<dyn Fn() -> bool + Send + Sync> = {
        let wakes = wakes.clone();
        Arc::new(move || wakes.wake())
    };
    // Declared after the worker, so a failed assertion releases the thread before the
    // worker joins it.
    let (release, teardown) = std::sync::mpsc::channel::<()>();
    let (_, events) = std::sync::mpsc::sync_channel(1);
    worker.image = Some(crate::image_view::ImageView::new(
        crate::image_worker::ImageWorker::waking_for_test(
            events,
            wake,
            Some(Box::new(move || {
                let _ = teardown.recv();
            })),
        ),
        worker.image_ids.clone(),
    ));
    worker.input(ViewerInput::NextFile, &output);
    worker
        .prefetch
        .as_ref()
        .unwrap()
        .serve(Err("captured identity: second.md".into()));
    worker.resume_open(&output);
    assert!(worker.pending_open && worker.capture_ready());
    assert_eq!(worker.status, "Closing previous image…");
    // The loop paints the opening's reset context in the same pass.
    worker.dirty = false;
    assert_eq!(worker.retry_delay(), None, "the worker blocks until a wake");
    assert!(receiver.try_recv().is_err(), "nothing woke it yet");
    release.send(()).unwrap();
    let Ok(Message::Ready) = receiver.recv_timeout(Duration::from_secs(10)) else {
        panic!("the closing actor's exit wakes the worker");
    };
    wakes.received();
    assert!(
        worker.closings_finished(),
        "the exit is published before its wake"
    );
    worker.resume_open(&output);
    assert!(!worker.pending_open, "the opening resumes on that wake");
    assert_eq!(worker.status, "captured identity: second.md");
    assert!(receiver.try_recv().is_err(), "exactly one wake");
}

#[test]
fn consecutive_resizes_in_one_batch_lay_out_once_with_the_final_geometry() {
    let (mut worker, output) = worker();
    prefer_wrap(&mut worker, &output, true);
    assert!(worker.set_page(text_page("wrapped words ".repeat(200)), &output));
    let (sender, receiver) = std::sync::mpsc::sync_channel(4);
    let wakes = crate::input_queue::Wakes::new(sender);
    assert!(wakes.wake());
    let resize = |width| {
        Message::Input(ViewerInput::Resize(ViewerGeometry {
            width,
            height: 100,
            scale: 1.0,
        }))
    };
    let generation = worker.positions.generation;
    let closed = AtomicBool::new(false);
    worker.input_batch(
        vec![resize(200), Message::Ready, resize(180), resize(160)],
        &closed,
        &wakes,
        &output,
    );
    assert_eq!(
        worker.positions.generation,
        generation + 1,
        "one layout context for the whole run of resizes"
    );
    assert_eq!(worker.geometry.width, 160);
    assert!(worker.shaped.as_ref().unwrap().width() <= 161.0);
    assert!(wakes.wake(), "Ready in the batch re-arms backend wakes");
    assert_eq!(receiver.try_iter().count(), 2);
    worker.input_batch(
        vec![
            resize(150),
            Message::Input(ViewerInput::Pan(0.0)),
            resize(140),
        ],
        &closed,
        &wakes,
        &output,
    );
    assert_eq!(
        worker.positions.generation,
        generation + 4,
        "another input between resizes keeps its own layout"
    );
    assert_eq!(worker.geometry.width, 140);
}

#[test]
fn scrollbar_seek_within_the_page_ends_a_cancelled_search_status() {
    let (mut worker, output) = worker();
    complete_page(&mut worker, &output, 80);
    frame(&mut worker, &output);
    worker.search_request = Some(7);
    worker.status = SEARCHING.into();
    worker.input(
        ViewerInput::Position {
            generation: worker.positions.generation,
            sequence: 1,
            horizontal: false,
            position: 0.5,
        },
        &output,
    );
    assert!(worker.search_request.is_none());
    assert!(
        worker.viewport_request.is_none(),
        "served from the loaded page"
    );
    let shown = frame(&mut worker, &output);
    assert_eq!(shown.position_sequence[0], 1);
    assert!(!shown.status.contains(SEARCHING), "{}", shown.status);
}

#[test]
fn retained_raster_choice_survives_an_image_for_the_next_text_file() {
    let (mut worker, output) = worker();
    worker.image = Some(crate::image_view::ImageView::new(
        crate::image_worker::ImageWorker::opened_for_test(
            vec![midnight_image_viewer_host::Metadata {
                width: 400,
                height: 600,
            }],
            false,
        ),
        worker.image_ids.clone(),
    ));
    worker.poll_image(&output);
    worker.input(ViewerInput::RetainedRaster(true), &output);
    assert!(worker.raster.enabled);
    worker.input(ViewerInput::RetainedRaster(false), &output);
    assert!(!worker.raster.enabled);
}

/// A request-scoped failure drained from the worker's session (generation 0).
fn failed(id: u64, code: u32, message: &str) -> midnight_plugin_host::HostEvent {
    midnight_plugin_host::HostEvent {
        request_id: id,
        generation: 0,
        kind: EventKind::Failed(messages::Failure {
            code,
            message: message.into(),
        }),
        fatal: false,
    }
}

/// The session-fatal failure plugin-host marks and drains first, naming request `id`.
fn fatal(id: u64, code: u32, message: &str) -> midnight_plugin_host::HostEvent {
    midnight_plugin_host::HostEvent {
        fatal: true,
        ..failed(id, code, message)
    }
}

fn completed(id: u64) -> midnight_plugin_host::HostEvent {
    midnight_plugin_host::HostEvent {
        request_id: id,
        generation: 0,
        kind: EventKind::Completed,
        fatal: false,
    }
}

#[test]
fn late_failures_of_untracked_requests_are_not_shown_and_not_watched() {
    let (mut worker, output) = worker();
    worker.session_events(
        vec![
            failed(9, INVALID, "late failure of a cancelled read"),
            failed(10, UNSUPPORTED, "optional request rejected"),
        ],
        &output,
    );
    assert!(
        output.take_open_result().is_none() && worker.status.is_empty(),
        "a request-scoped failure of an untracked request leaves the session usable"
    );
    assert_eq!(worker.retry_delay(), None, "nothing is polled afterwards");
}

#[test]
fn untracked_fatal_failure_ahead_of_a_full_drain_is_reported_with_its_message() {
    let (mut worker, output) = worker();
    worker.viewport_request = Some(41);
    worker.search_request = Some(42);
    worker.status = SEARCHING.into();
    // The first 32-event drain after plugin-host stopped the session on output of a
    // cancelled request (9) while more than 32 older events were queued: the fatal
    // failure leads, and output queued before it follows, including terminal events of
    // requests that completed before the failure. The host still holds more of it.
    let mut events = vec![
        fatal(9, INVALID, "Malformed plugin output"),
        failed(41, INVALID, "Plugin rejected request (3)"),
        failed(
            OPEN_REQUEST_ID,
            INVALID,
            "older failure of the Open request",
        ),
    ];
    events.extend((100..).map(completed).take(DRAIN - events.len()));
    assert_eq!(events.len(), DRAIN);
    worker.session_events(events, &output);
    assert_eq!(worker.status, "Malformed plugin output");
    assert_eq!(
        output.take_open_result(),
        Some(Err("Malformed plugin output".into()))
    );
    assert!(
        worker.viewport_request.is_none()
            && worker.search_request.is_none()
            && worker.continuation.is_none(),
        "requests that waited on the session end with it"
    );
    // The loop paints the reset context in the same pass. Nothing then waits for the
    // failed session's end by polling.
    worker.dirty = false;
    assert_eq!(worker.retry_delay(), None);
}

#[test]
fn tracked_and_session_level_fatal_failures_are_reported_the_same_way() {
    for (event, request) in [
        (
            fatal(41, INVALID, "Plugin output does not match captured content"),
            Some(41),
        ),
        (
            fatal(OPEN_REQUEST_ID, QUEUE_FULL, "Viewer output budget exceeded"),
            None,
        ),
    ] {
        let (mut worker, output) = worker();
        worker.viewport_request = request;
        let EventKind::Failed(failure) = &event.kind else {
            unreachable!()
        };
        let message = failure.message.clone();
        worker.session_events(vec![event, completed(41)], &output);
        assert_eq!(worker.status, message);
        assert_eq!(output.take_open_result(), Some(Err(message)));
        assert!(worker.viewport_request.is_none());
    }
}

#[test]
fn fatal_failure_naming_the_optional_locate_request_is_reported_at_once() {
    const MESSAGE: &str = "Plugin output does not match captured content";
    for code in [INVALID, QUEUE_FULL, UNSUPPORTED] {
        let (mut worker, output) = worker();
        worker.text.locate_request = Some((12, 0));
        worker.session_events(vec![failed(12, code, MESSAGE)], &output);
        assert!(worker.text.locate_request.is_none());
        assert_eq!(worker.text.unavailable, code != QUEUE_FULL);
        assert!(
            worker.status.is_empty(),
            "an optional lookup's own failure leaves the page usable (code {code})"
        );
    }
    let (mut worker, output) = worker();
    worker.text.locate_request = Some((12, 0));
    worker.session_events(vec![fatal(12, INVALID, MESSAGE)], &output);
    assert_eq!(worker.status, MESSAGE);
    assert!(worker.text.locate_request.is_none());
}

#[test]
fn source_gutter_shapes_only_numbers_near_the_painted_rows() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    complete_page(&mut worker, &output, 400);
    frame(&mut worker, &output);
    let shaped = |worker: &Worker| {
        let lines = worker.text.gutter.as_ref().unwrap();
        (
            lines.len(),
            lines.iter().filter(|line| line.label.is_some()).count(),
        )
    };
    let (lines, labels) = shaped(&worker);
    assert!(lines >= 400);
    assert!(labels > 0 && labels < 20, "{labels} numbers shaped");
    let bottom = worker.shaped.as_ref().unwrap().height() - worker.geometry.height as f32;
    worker.input(ViewerInput::Scroll { x: 0.0, y: bottom }, &output);
    frame(&mut worker, &output);
    let last = worker.text.gutter.as_ref().unwrap().last().unwrap();
    assert!(last.label.is_some(), "rows scrolled into view are numbered");
    assert!(shaped(&worker).1 < 40);
}

#[test]
fn select_page_selects_the_visible_rows_without_retained_pixels() {
    let (mut worker, output) = worker();
    complete_page(&mut worker, &output, 80);
    worker.origin[1] = 200.0;
    let revision = frame(&mut worker, &output).revision;
    worker.input(ViewerInput::SelectPage { frame: revision }, &output);
    let (first, last) = worker.selection.unwrap();
    let page = worker.shaped.as_ref().unwrap();
    assert_eq!(
        (first, last),
        visible_rows(page, worker.origin[1], worker.geometry)
    );
    assert!(first > 0 && last < page.text().len(), "{first}..{last}");
}
