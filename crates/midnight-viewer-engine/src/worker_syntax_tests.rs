//! Exercise real worker event handling; only transport-assigned IDs are supplied.
use super::*;

#[path = "worker_source_overlap_tests.rs"]
mod overlap;

fn viewport(anchor: u64) -> messages::Viewport {
    messages::Viewport {
        source_anchor: anchor,
        backwards: false,
        max_bytes: 12 * 1024,
        max_lines: 64,
    }
}

fn decoded(styled: bool) -> viewer::TextPage {
    let text = "int selected_value = 42; // generated\n".repeat(18);
    viewer::TextPage {
        revision: 9,
        source: viewer::SourceRange {
            offset: 8192,
            length: text.len() as u64,
        },
        anchors: text
            .char_indices()
            .map(|(at, _)| viewer::Anchor {
                text_offset: at as u32,
                source_offset: 8192 + at as u64,
            })
            .chain(std::iter::once(viewer::Anchor {
                text_offset: text.len() as u32,
                source_offset: 8192 + text.len() as u64,
            }))
            .collect(),
        spans: if styled {
            vec![viewer::Span {
                start: 0,
                end: text.len() as u32,
                role: viewer::STYLE_STRING,
                flags: 0,
            }]
        } else {
            Vec::new()
        },
        text,
        has_before: true,
        has_after: true,
        partial_line: false,
    }
}

fn source_worker() -> (Worker, Mailbox) {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    worker.generation = 7;
    worker.length = 128 * 1024;
    assert!(worker.set_page(Page::text(decoded(false), &worker.appearance), &output));
    worker.origin = [0.0, 24.0];
    worker.selection = Some((4, 18));
    worker.text.syntax.demand(7, 9, viewport(8192));
    (worker, output)
}

fn ready(worker: &mut Worker, output: &Mailbox, id: u64) {
    let key = worker.text.syntax.latest.clone().unwrap();
    worker.text.syntax.started(id, key);
    worker.event(
        id,
        EventKind::SourcePrepared(messages::SourcePrepared {
            revision: 9,
            frontier: 8192,
            ready: true,
        }),
        output,
    );
    assert!(
        worker.text.syntax.refresh.is_none(),
        "typed result is provisional"
    );
    worker.event(id, EventKind::Completed, output);
}

#[test]
fn latest_stationary_target_alone_can_authorize_one_refresh() {
    let (mut worker, output) = source_worker();
    let original = worker.text.syntax.latest.clone().unwrap();
    worker.text.syntax.started(41, original);
    worker.text.syntax.demand(7, 9, viewport(48_000));
    worker.event(
        41,
        EventKind::SourcePrepared(messages::SourcePrepared {
            revision: 9,
            frontier: 8192,
            ready: true,
        }),
        &output,
    );
    worker.event(41, EventKind::Completed, &output);
    assert!(worker.text.syntax.refresh.is_none());
    assert_eq!(
        worker
            .text
            .syntax
            .latest
            .as_ref()
            .unwrap()
            .viewport
            .source_anchor,
        48_000
    );

    ready(&mut worker, &output, 42);
    assert_eq!(
        worker
            .text
            .syntax
            .refresh
            .as_ref()
            .unwrap()
            .key
            .viewport
            .source_anchor,
        48_000
    );
    worker.text.syntax.refresh_admitted(43);
    worker.event(43, EventKind::TextPage(decoded(true)), &output);
    worker.event(43, EventKind::Completed, &output);
    assert!(worker.text.syntax.refresh.is_none());
    assert!(
        !worker
            .text
            .syntax
            .needs_preparation(std::time::Instant::now())
    );
    worker.event(42, EventKind::Completed, &output);
    assert!(
        worker.text.syntax.refresh.is_none(),
        "late terminal cannot create a loop"
    );
}

#[test]
fn successful_restyle_preserves_selection_origin_and_navigation_acknowledgements() {
    for retained in [false, true] {
        let (mut worker, output) = source_worker();
        worker.raster.enabled = retained;
        if retained {
            worker.reset_raster_context();
        }
        worker.paint(&output).unwrap();
        let first = output
            .take()
            .into_iter()
            .find_map(|event| match event {
                ViewerOutput::Frame(frame) => Some(frame),
                _ => None,
            })
            .unwrap();
        let old_origin = worker.origin;
        let old_selection = worker.selection;
        let old_context = worker.positions.generation;
        let old_ack = worker.positions.acknowledged;
        ready(&mut worker, &output, 41);
        worker.text.syntax.refresh_admitted(42);
        worker.event(42, EventKind::TextPage(decoded(true)), &output);
        assert!(
            worker.page.as_ref().unwrap().spans.is_empty(),
            "keep old page until Completed"
        );
        worker.event(42, EventKind::Completed, &output);
        assert!(!worker.page.as_ref().unwrap().spans.is_empty());
        assert_eq!(worker.selection, old_selection);
        assert_eq!(worker.origin, old_origin);
        assert_eq!(worker.positions.generation, old_context);
        assert_eq!(worker.positions.acknowledged, old_ack);
        assert!(worker.viewport_request.is_none());
        worker.paint(&output).unwrap();
        let second = output
            .take()
            .into_iter()
            .find_map(|event| match event {
                ViewerOutput::Frame(frame) => Some(frame),
                _ => None,
            })
            .unwrap();
        assert_eq!(second.accessible_text, first.accessible_text);
        // Retained frames carry their pixels as prepared strips.
        let shown = |frame: &ViewerFrame| {
            if frame.raster.is_some() {
                super::raster::stacked(frame).1
            } else {
                frame.pixels.to_vec()
            }
        };
        assert_ne!(
            shown(second.as_ref()),
            shown(first.as_ref()),
            "new styles must reach pixels"
        );
        assert_eq!(second.position_sequence, first.position_sequence);
    }
}

#[test]
fn navigation_during_refresh_discards_its_page_without_changing_the_new_view() {
    let (mut worker, output) = source_worker();
    ready(&mut worker, &output, 41);
    worker.text.syntax.refresh_admitted(42);
    worker.invalidate_source_target();
    let origin = worker.origin;
    worker.event(42, EventKind::TextPage(decoded(true)), &output);
    worker.event(42, EventKind::Completed, &output);
    assert!(worker.page.as_ref().unwrap().spans.is_empty());
    assert_eq!(worker.origin, origin);
    assert_eq!(worker.selection, Some((4, 18)));
}

#[test]
fn unsupported_is_cached_for_the_session_but_temporary_failure_is_retried_with_backoff() {
    use midnight_plugin_host::api::abi;
    for code in [abi::UNSUPPORTED, abi::QUEUE_FULL, abi::PANICKED] {
        let (mut worker, output) = source_worker();
        let key = worker.text.syntax.latest.clone().unwrap();
        worker.text.syntax.started(41, key);
        worker.event(
            41,
            EventKind::Failed(messages::Failure {
                code,
                message: "generated preparation failure".into(),
            }),
            &output,
        );
        assert_eq!(worker.text.syntax.unavailable, code == abi::UNSUPPORTED);
        assert!(worker.text.syntax.refresh.is_none());
        assert!(worker.page.as_ref().unwrap().spans.is_empty());
        if code == abi::QUEUE_FULL {
            let now = std::time::Instant::now();
            assert!(!worker.text.syntax.needs_preparation(now), "no hot loop");
            assert!(
                worker
                    .text
                    .syntax
                    .needs_preparation(now + Duration::from_secs(1))
            );
        } else {
            assert!(
                !worker
                    .text
                    .syntax
                    .needs_preparation(std::time::Instant::now())
            );
        }
        if code == abi::PANICKED {
            assert_eq!(worker.status, "generated preparation failure");
        }
    }
}

#[test]
fn cancellation_missing_result_and_close_never_authorize_a_refresh() {
    for scenario in 0..3 {
        let (mut worker, output) = source_worker();
        let key = worker.text.syntax.latest.clone().unwrap();
        worker.text.syntax.started(41, key);
        if scenario != 1 {
            worker.event(
                41,
                EventKind::SourcePrepared(messages::SourcePrepared {
                    revision: 9,
                    frontier: 8192,
                    ready: true,
                }),
                &output,
            );
        }
        match scenario {
            0 => worker.event(41, EventKind::Cancelled, &output),
            1 => worker.event(41, EventKind::Completed, &output),
            _ => {
                worker.input(ViewerInput::Close, &output);
                worker.event(41, EventKind::Completed, &output);
            }
        }
        assert!(worker.text.syntax.refresh.is_none());
        assert!(
            !worker
                .text
                .syntax
                .needs_preparation(std::time::Instant::now())
        );
        assert!(worker.page.as_ref().unwrap().spans.is_empty());
    }
}

#[test]
fn equal_frontier_is_valid_progress_and_foreground_always_blocks_preparation() {
    let (mut worker, output) = source_worker();
    for id in [41, 42] {
        let key = worker.text.syntax.latest.clone().unwrap();
        worker.text.syntax.started(id, key);
        worker.event(
            id,
            EventKind::SourcePrepared(messages::SourcePrepared {
                revision: 9,
                frontier: 8192,
                ready: false,
            }),
            &output,
        );
        worker.event(id, EventKind::Completed, &output);
        assert!(
            worker
                .text
                .syntax
                .needs_preparation(std::time::Instant::now())
        );
    }
    worker.viewport_request = Some(43);
    assert!(!worker.source_preparation_admissible());
    worker.viewport_request = None;
    assert!(worker.source_preparation_admissible());
}

#[test]
fn stale_unsupported_result_still_caches_absence_without_clearing_a_newer_status() {
    let (mut worker, output) = source_worker();
    let old = worker.text.syntax.latest.clone().unwrap();
    worker.text.syntax.started(41, old);
    worker.invalidate_source_target();
    worker.status("newer visible status".into(), &output);
    worker.event(
        41,
        EventKind::Failed(messages::Failure {
            code: midnight_plugin_host::api::abi::UNSUPPORTED,
            message: "old plugin".into(),
        }),
        &output,
    );
    assert!(worker.text.syntax.unavailable);
    assert_eq!(worker.status, "newer visible status");
}

#[test]
fn locate_only_caches_unsupported_and_paces_temporary_capacity_failure() {
    use midnight_plugin_host::api::abi;
    for code in [abi::UNSUPPORTED, abi::QUEUE_FULL, abi::PANICKED] {
        let (mut worker, output) = source_worker();
        worker.text.locate_request = Some((44, 8192));
        worker.event(
            44,
            EventKind::Failed(messages::Failure {
                code,
                message: "generated locate failure".into(),
            }),
            &output,
        );
        assert_eq!(worker.text.locate_unsupported, code == abi::UNSUPPORTED);
        if code == abi::QUEUE_FULL {
            assert!(
                worker
                    .text
                    .locate_retry_at
                    .is_some_and(|retry| retry > std::time::Instant::now())
            );
            assert!(!worker.text.unavailable);
        }
    }
}

/// A refresh that holds fewer rows than the page restyles the rows it holds: the page keeps
/// its rows, its selection, the visible source and the retained camera. Before, the page was
/// replaced by the refresh's rows.
#[test]
fn cropped_dense_restyle_keeps_exact_source_selection_visual_anchor_and_retained_camera() {
    let (mut worker, output) = source_worker();
    worker.raster.enabled = true;
    let row = "int selected_value = 42; // generated\n".len();
    worker.text.syntax.demand(
        7,
        9,
        messages::Viewport {
            source_anchor: decoded(false).source.end().unwrap(),
            backwards: true,
            ..viewport(8192)
        },
    );
    worker.origin = [0.0, 120.0];
    worker.selection = Some((row * 8 + 4, row * 8 + 18));
    worker.reset_raster_context();
    worker.paint(&output).unwrap();
    let first = output
        .take()
        .into_iter()
        .find_map(|event| match event {
            ViewerOutput::Frame(frame) => Some(frame),
            _ => None,
        })
        .unwrap();
    let first_raster = first.raster.unwrap();
    let source_caret = |worker: &Worker| {
        let shaped = worker.shaped.as_ref().unwrap();
        let at = shaped
            .hit_test(worker.origin[0], worker.origin[1] + 0.5)
            .unwrap();
        let source = worker
            .page
            .as_ref()
            .unwrap()
            .anchors
            .iter()
            .find(|anchor| anchor.text_offset as usize == at)
            .unwrap()
            .source_offset;
        let caret = shaped.caret_geometry(at).unwrap();
        (source, worker.origin[1] - caret.y0 as f32)
    };
    let visible = source_caret(&worker);
    let copied = worker
        .shaped
        .as_ref()
        .unwrap()
        .selected_text(row * 8 + 4..row * 8 + 18)
        .unwrap()
        .to_owned();
    let acknowledged = worker.positions.acknowledged;
    let generation = worker.positions.generation;
    let mut cropped = decoded(false);
    let removed = row * 3;
    cropped.text.drain(..removed);
    cropped.source.offset += removed as u64;
    cropped.source.length -= removed as u64;
    cropped
        .anchors
        .retain(|anchor| anchor.text_offset as usize >= removed);
    for anchor in &mut cropped.anchors {
        anchor.text_offset -= removed as u32;
    }
    cropped.spans = (0..cropped.text.len())
        .step_by(2)
        .map(|at| viewer::Span {
            start: at as u32,
            end: (at + 1) as u32,
            role: viewer::STYLE_KEYWORD,
            flags: 0,
        })
        .collect();
    ready(&mut worker, &output, 41);
    worker.text.syntax.refresh_admitted(42);
    worker.event(42, EventKind::TextPage(cropped), &output);
    worker.event(42, EventKind::Completed, &output);
    let page = worker.page.as_ref().unwrap();
    assert_eq!(page.start, 8192);
    assert!(!page.spans.is_empty());
    assert!(page.spans.iter().all(|span| span.range.start >= removed));
    assert_eq!(source_caret(&worker), visible);
    assert_eq!(worker.selection, Some((row * 8 + 4, row * 8 + 18)));
    assert_eq!(
        worker
            .shaped
            .as_ref()
            .unwrap()
            .selected_text(row * 8 + 4..row * 8 + 18)
            .unwrap(),
        copied
    );
    assert_eq!(worker.positions.acknowledged, acknowledged);
    assert_eq!(worker.positions.generation, generation);
    worker.paint(&output).unwrap();
    let second = output
        .take()
        .into_iter()
        .find_map(|event| match event {
            ViewerOutput::Frame(frame) => Some(frame),
            _ => None,
        })
        .unwrap();
    let second_raster = second.raster.unwrap();
    assert_eq!(second_raster.surface, first_raster.surface);
    assert_eq!(second_raster.origin, first_raster.origin);
    assert_eq!(second_raster.pan_sequence, first_raster.pan_sequence);
    assert!(second_raster.pixel_id > first_raster.pixel_id);
    assert_eq!(second.position_sequence, first.position_sequence);
}

#[test]
fn capacity_held_past_eight_attempts_keeps_one_intent_then_refreshes_once_when_released() {
    use midnight_plugin_host::AdmissionError;
    let (mut worker, output) = source_worker();
    ready(&mut worker, &output, 41);
    let key = worker.text.syntax.refresh.as_ref().unwrap().key.clone();
    let selection = worker.selection;
    let origin = worker.origin;
    for _ in 0..20 {
        worker.source_refresh_admission(&key, Err(AdmissionError::Capacity), &output);
        let now = std::time::Instant::now();
        assert!(
            !worker.text.syntax.refresh_due(now),
            "paced; no immediate requeue"
        );
        assert!(worker.text.syntax.refresh_due(now + Duration::from_secs(1)));
        assert!(
            worker.text.syntax.refresh.is_some(),
            "one intent survives long saturation"
        );
        assert!(
            !worker
                .text
                .syntax
                .needs_preparation(now + Duration::from_secs(1))
        );
        assert_eq!(worker.positions.acknowledged, [0; 2]);
    }
    worker.source_refresh_admission(&key, Ok(42), &output);
    worker.event(42, EventKind::TextPage(decoded(true)), &output);
    worker.event(42, EventKind::Completed, &output);
    assert!(!worker.page.as_ref().unwrap().spans.is_empty());
    assert_eq!(worker.origin, origin);
    assert_eq!(worker.selection, selection);
    assert!(worker.text.syntax.refresh.is_none());
    assert!(
        !worker
            .text
            .syntax
            .needs_preparation(std::time::Instant::now() + Duration::from_secs(2))
    );
    worker.event(42, EventKind::Completed, &output);
    assert!(worker.text.syntax.refresh.is_none());
}

#[test]
fn local_noncapacity_refresh_rejections_settle_the_epoch_without_blanking_the_page() {
    use midnight_plugin_host::{AdmissionError, Error};
    for admission in [
        AdmissionError::Closing,
        AdmissionError::Invalid(Error("invalid captured request".into())),
    ] {
        let (mut worker, output) = source_worker();
        ready(&mut worker, &output, 41);
        let key = worker.text.syntax.refresh.as_ref().unwrap().key.clone();
        worker.source_refresh_admission(&key, Err(admission), &output);
        assert!(worker.text.syntax.refresh.is_none());
        assert!(
            !worker
                .text
                .syntax
                .needs_preparation(std::time::Instant::now() + Duration::from_secs(1))
        );
        assert_eq!(
            worker.page.as_ref().unwrap().text.as_ref(),
            decoded(false).text.as_str()
        );
    }
}

/// The notice of rows whose highlighting is being prepared waits until the visible rows have
/// stayed plain at rest for `NOTICE_DELAY`, and goes once they are coloured.
#[test]
fn the_notice_waits_for_plain_visible_rows_at_rest_and_goes_when_they_are_coloured() {
    use super::super::syntax::{self, NOTICE_DELAY, PREPARING_HIGHLIGHTING};
    let (mut worker, output) = source_worker();
    worker.length = 1 << 20;
    worker.selection = None;
    syntax::set_clock(Some(std::time::Instant::now()));
    // The first rest chooses the target; the delay counts from the rest after it.
    worker.prepare_source(&output);
    worker.prepare_source(&output);
    assert!(worker.visible_rows_plain());
    syntax::advance_clock(NOTICE_DELAY - Duration::from_millis(1));
    worker.prepare_source(&output);
    assert!(worker.status.is_empty(), "{}", worker.status);
    syntax::advance_clock(Duration::from_millis(1));
    worker.prepare_source(&output);
    assert_eq!(worker.status, PREPARING_HIGHLIGHTING);
    ready(&mut worker, &output, 41);
    worker.text.syntax.refresh_admitted(42);
    worker.event(42, EventKind::TextPage(decoded(true)), &output);
    worker.event(42, EventKind::Completed, &output);
    assert!(worker.status.is_empty(), "{}", worker.status);
    syntax::advance_clock(NOTICE_DELAY * 2);
    worker.prepare_source(&output);
    assert!(worker.status.is_empty(), "{}", worker.status);
}

/// While the camera moves the notice stays away, however long the rows it shows stay plain.
#[test]
fn the_notice_never_shows_while_the_camera_moves() {
    use super::super::syntax;
    let (mut worker, output) = source_worker();
    worker.length = 1 << 20;
    worker.selection = None;
    let mut whole = decoded(false);
    (whole.has_before, whole.has_after) = (false, false);
    assert!(worker.set_page(Page::text(whole, &worker.appearance), &output));
    syntax::set_clock(Some(std::time::Instant::now()));
    for step in 0..60 {
        let y = if step % 2 == 0 { 3.0 } else { -3.0 };
        worker.input(ViewerInput::Scroll { x: 0.0, y }, &output);
        worker.prepare_source(&output);
        assert!(worker.visible_rows_plain());
        assert!(worker.status.is_empty(), "frame {step}: {}", worker.status);
        syntax::advance_clock(Duration::from_nanos(16_666_667));
    }
}

/// A document within one walk is prepared whole once, from its end back: every page read from
/// it then comes coloured, and the rows that came plain before are only read again.
#[test]
fn a_document_within_one_walk_is_prepared_whole_once_then_only_refreshed() {
    use super::super::syntax::{MAX_SCAN_BYTES, SourceAdmission};
    let (mut worker, output) = source_worker();
    worker.selection = None;
    assert!(worker.length <= u64::from(MAX_SCAN_BYTES));
    let (key, admission) = worker.source_admission(&output).unwrap();
    let SourceAdmission::Prepare(request) = &admission else {
        panic!("a preparation first");
    };
    assert_eq!(
        (
            request.viewport.source_anchor,
            request.viewport.backwards,
            request.max_scan_bytes
        ),
        (worker.length, true, MAX_SCAN_BYTES)
    );
    worker.source_admitted(&key, &admission, Ok(41), &output);
    worker.event(
        41,
        EventKind::SourcePrepared(messages::SourcePrepared {
            revision: 9,
            frontier: worker.length,
            ready: true,
        }),
        &output,
    );
    worker.event(41, EventKind::Completed, &output);
    assert!(worker.text.syntax.refresh.is_none(), "nothing to read yet");
    let (key, admission) = worker.source_admission(&output).unwrap();
    assert!(
        matches!(admission, SourceAdmission::Refresh(_)),
        "the plain rows are read again, not prepared"
    );
    worker.source_admitted(&key, &admission, Ok(42), &output);
    worker.event(42, EventKind::TextPage(decoded(true)), &output);
    worker.event(42, EventKind::Completed, &output);
    assert!(!worker.page.as_ref().unwrap().spans.is_empty());
    assert!(worker.source_admission(&output).is_none());
}

/// A refresh that completes while the view is not at rest (here a page request is under way)
/// keeps its page until the view rests, and then colours the rows where they are.
#[test]
fn a_refresh_completed_away_from_rest_colours_its_rows_at_the_next_rest() {
    let (mut worker, output) = source_worker();
    worker.length = 1 << 20;
    worker.selection = None;
    let origin = worker.origin;
    ready(&mut worker, &output, 41);
    worker.text.syntax.refresh_admitted(42);
    worker.viewport_request = Some(99);
    worker.event(42, EventKind::TextPage(decoded(true)), &output);
    worker.event(42, EventKind::Completed, &output);
    assert!(
        worker.page.as_ref().unwrap().spans.is_empty(),
        "not at rest"
    );
    assert!(worker.text.syntax.refresh.is_some(), "the page waits");
    worker.viewport_request = None;
    assert!(worker.source_admission(&output).is_none());
    assert!(!worker.page.as_ref().unwrap().spans.is_empty());
    assert!(worker.text.syntax.refresh.is_none());
    assert_eq!(worker.origin, origin);
}
