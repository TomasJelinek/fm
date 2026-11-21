use super::*;

#[test]
fn source_overlapping_seek_chain_keeps_known_lines_when_first_column_is_unknown() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    let source: String = (0..90)
        .map(|i| format!("{{\"row\":{i},\"name\":\"東京\"}}\n"))
        .collect();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    worker.length = source.len() as u64;
    for (start, end) in [
        (0, starts[40]),
        (starts[20] + 5, starts[60]),
        (starts[10] + 7, starts[50]),
        (starts[2] + 3, starts[18]),
    ] {
        assert!(worker.set_page(source_page(&source, start, end), &output));
    }
    worker.prepare_gutter().unwrap();
    assert_eq!(
        worker
            .text
            .gutter
            .as_ref()
            .unwrap()
            .first()
            .expect("known lines survive even when column prefix is outside overlap")
            .line,
        3
    );
}

#[test]
fn source_unwrapped_bounded_fragment_navigation_always_makes_progress() {
    let (mut worker, _) = worker();
    let source = format!("prior\n{}tail", "word".repeat(2048));
    let page = source_page(&source, 6, 6 + 8192);
    let shaped = worker.layout_page(&page).unwrap();
    assert!(shaped.height() < worker.geometry.height as f32);
    let forward = crate::viewport::Continuation::capture(
        &page,
        &shaped,
        worker.geometry,
        [0.0; 2],
        17.0,
        true,
    )
    .unwrap();
    assert_eq!(
        forward.anchor, page.end,
        "a huge logical line must not reload its first fragment"
    );
    let backward = crate::viewport::Continuation::capture(
        &page,
        &shaped,
        worker.geometry,
        [0.0; 2],
        -17.0,
        true,
    )
    .unwrap();
    assert_eq!(
        backward.anchor, page.start,
        "backward endpoint excludes the current bounded fragment"
    );
}

#[test]
fn source_page_refills_before_exposing_its_unloaded_last_visual_line() {
    for trailing_newline in [false, true] {
        let (mut worker, _) = worker();
        let mut source: String = (0..40).map(|i| format!("row {i}\n")).collect();
        if !trailing_newline {
            source.pop();
        }
        let mut page = source_page(&source, 0, source.len());
        page.after = true;
        let shaped = worker.layout_page(&page).unwrap();
        let (_, last_top) = shaped.line_at_y(shaped.height()).unwrap();
        let geometry = ViewerGeometry {
            width: 240,
            height: 100,
            scale: 1.0,
        };
        let origin = [0.0, last_top - geometry.height as f32 - 1.0];
        assert!(
            crate::viewport::Continuation::capture(&page, &shaped, geometry, origin, 2.0, true)
                .is_some(),
            "a not-yet-loaded row must not enter the displayed viewport (newline={trailing_newline})"
        );
        page.after = false;
        assert!(
            crate::viewport::Continuation::capture(&page, &shaped, geometry, origin, 2.0, true)
                .is_none(),
            "real EOF keeps its ordinary final line"
        );
    }
}

#[test]
fn source_empty_optional_locate_is_per_page_and_near_coordinates_can_recover() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    assert!(worker.set_page(text_page("far fragment".into()), &output));
    worker.text.locate_request = Some((77, 8000));
    worker.event(77, EventKind::Completed, &output);
    assert!(worker.text.unavailable);
    assert_eq!(worker.text.located_start, Some(8000));
    let source = "first\nsecond\nthird\n";
    assert!(worker.set_page(source_page(source, 6, source.len()), &output));
    assert!(
        worker
            .source_status(
                worker.page.as_ref().unwrap(),
                worker.shaped.as_ref().unwrap()
            )
            .contains("indexing"),
        "a later page must retry its cheap optional lookup"
    );
    worker.text.locate_request = Some((78, 6));
    worker.event(
        78,
        EventKind::TextPosition(viewer::TextPosition {
            revision: 1,
            source_offset: 6,
            line: 2,
            column: 1,
        }),
        &output,
    );
    worker.event(78, EventKind::Completed, &output);
    assert!(
        worker
            .source_status(
                worker.page.as_ref().unwrap(),
                worker.shaped.as_ref().unwrap()
            )
            .starts_with("Ln 2, Col 1")
    );
}

fn source_page(source: &str, start: usize, end: usize) -> Page {
    let text = source[start..end].to_owned();
    Page::text(
        viewer::TextPage {
            revision: 1,
            source: viewer::SourceRange {
                offset: start as u64,
                length: (end - start) as u64,
            },
            anchors: text
                .char_indices()
                .map(|(at, _)| viewer::Anchor {
                    text_offset: at as u32,
                    source_offset: (start + at) as u64,
                })
                .chain(std::iter::once(viewer::Anchor {
                    text_offset: text.len() as u32,
                    source_offset: end as u64,
                }))
                .collect(),
            text,
            spans: Vec::new(),
            has_before: start != 0,
            has_after: end != source.len(),
            partial_line: false,
        },
        &appearance(),
    )
}

fn appearance() -> ViewerAppearance {
    ViewerAppearance {
        family: "monospace".into(),
        font_size: 14.0,
        foreground: [31, 35, 40, 255],
        background: [255; 4],
        selection: [180, 210, 250, 255],
        accent: [9, 105, 218, 255],
    }
}

#[test]
fn source_scroll_first_frame_keeps_exact_overlap_line_numbers_without_locate() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    let source: String = (0..90)
        .map(|i| format!("{{\"row\":{i},\"name\":\"東京\"}}\n"))
        .collect();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    worker.length = source.len() as u64;
    assert!(worker.set_page(source_page(&source, 0, starts[40]), &output));
    assert!(worker.set_page(source_page(&source, starts[5], starts[45]), &output));
    let coordinate = worker
        .text
        .coordinate
        .expect("overlap keeps its known source coordinate");
    assert_eq!(
        (coordinate.source_offset, coordinate.line, coordinate.column),
        (starts[5] as u64, 6, Some(1))
    );
    worker.prepare_gutter().unwrap();
    let labels = worker.text.gutter.as_ref().unwrap();
    assert_eq!(labels.first().unwrap().line, 6);
    assert!(labels.len() >= 40);
    assert!(worker.set_page(source_page(&source, starts[2], starts[42]), &output));
    worker.prepare_gutter().unwrap();
    assert_eq!(
        worker.text.gutter.as_ref().unwrap().first().unwrap().line,
        3
    );
}

fn continuation_fixture() -> (Worker, Mailbox, String, Vec<usize>) {
    let (mut worker, output) = worker();
    let source: String = (0..90)
        .map(|i| format!("row {i:03} {}\n", "wide text ".repeat(10)))
        .collect();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    worker.length = source.len() as u64;
    assert!(worker.set_page(source_page(&source, starts[10], starts[30]), &output));
    let maximum = crate::viewport::available_height(
        worker.page.as_ref().unwrap(),
        worker.shaped.as_ref().unwrap(),
        worker.geometry.height as f32,
    ) - worker.geometry.height as f32;
    worker.origin = [0.0, maximum - 0.25];
    (worker, output, source, starts)
}

#[test]
fn continuation_preserves_reversed_motion_while_the_next_fragment_is_pending() {
    let (mut worker, output, source, starts) = continuation_fixture();
    let screen_y = |worker: &Worker| {
        let index = worker
            .page
            .as_ref()
            .unwrap()
            .anchors
            .iter()
            .find(|anchor| anchor.source_offset == starts[22] as u64)
            .unwrap()
            .text_offset;
        worker
            .shaped
            .as_ref()
            .unwrap()
            .caret_geometry(index as usize)
            .unwrap()
            .y0 as f32
            - worker.origin[1]
    };
    let before = screen_y(&worker);
    worker.input(ViewerInput::Scroll { x: 0.0, y: 3.5 }, &output);
    assert!(worker.continuation.is_some());
    // The fixture supplies the asynchronous response without requiring an installed plugin.
    worker.viewport_request = Some(41);
    worker.input(ViewerInput::Scroll { x: 0.0, y: -1.25 }, &output);
    worker.received_page(41, source_page(&source, starts[15], starts[45]), &output);
    assert!(
        (before - screen_y(&worker)).abs() < 0.001,
        "a staged page cannot apply either crossing or queued reversal before completion"
    );
    worker.event(40, EventKind::Completed, &output);
    assert_eq!(worker.viewport_request, Some(41));
    assert!(
        (before - screen_y(&worker)).abs() < 0.001,
        "an unrelated completion cannot consume the pending camera or reversal"
    );
    worker.event(41, EventKind::Completed, &output);
    let moved = before - screen_y(&worker);
    assert!(
        (moved - 2.25).abs() < 0.001,
        "reversal was overwritten by the page reply: {moved}"
    );
}

#[test]
fn continuation_discards_staged_page_and_reversal_after_cancellation_or_failure() {
    for terminal in [
        EventKind::Cancelled,
        EventKind::Failed(messages::Failure {
            code: midnight_plugin_host::api::abi::INVALID,
            message: "controlled continuation rejection".into(),
        }),
    ] {
        let (mut worker, output, source, starts) = continuation_fixture();
        let origin = worker.origin;
        let previous = worker.page.as_ref().unwrap().text.clone();
        worker.input(ViewerInput::Scroll { x: 0.0, y: 3.5 }, &output);
        worker.viewport_request = Some(41);
        worker.input(ViewerInput::Scroll { x: 0.0, y: -1.25 }, &output);
        worker.received_page(41, source_page(&source, starts[15], starts[45]), &output);
        assert_eq!(worker.origin, origin, "Page is not a completion");
        assert_eq!(worker.page.as_ref().unwrap().text, previous);

        worker.event(41, terminal, &output);
        assert!(worker.viewport_request.is_none());
        assert_eq!(
            worker.origin, origin,
            "a failed read cannot apply queued motion"
        );
        assert_eq!(worker.page.as_ref().unwrap().text, previous);
        let late = source_page(&source, starts[15], starts[45]);
        let late = viewer::TextPage {
            revision: 1,
            source: viewer::SourceRange {
                offset: late.start,
                length: late.end - late.start,
            },
            text: late.text.as_ref().into(),
            anchors: late.anchors,
            spans: Vec::new(),
            has_before: late.before,
            has_after: late.after,
            partial_line: false,
        };
        late.validate().unwrap();
        worker.event(41, EventKind::TextPage(late), &output);
        worker.event(41, EventKind::Completed, &output);
        assert_eq!(
            worker.origin, origin,
            "retired callbacks cannot revive the tail"
        );
        assert_eq!(worker.page.as_ref().unwrap().text, previous);
        worker.input(ViewerInput::Scroll { x: 0.0, y: -0.5 }, &output);
        assert_eq!(worker.origin, [origin[0], origin[1] - 0.5]);
    }
}

#[test]
fn continuation_keeps_horizontal_pixels_from_the_crossing_gesture() {
    let (mut worker, output) = worker();
    let source: String = (0..90)
        .map(|i| format!("row {i:03} {}\n", "wide text ".repeat(10)))
        .collect();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    worker.length = source.len() as u64;
    assert!(worker.set_page(source_page(&source, starts[10], starts[30]), &output));
    worker.origin = [
        10.0,
        crate::viewport::available_height(
            worker.page.as_ref().unwrap(),
            worker.shaped.as_ref().unwrap(),
            worker.geometry.height as f32,
        ) - worker.geometry.height as f32
            - 0.25,
    ];
    worker.input(ViewerInput::Scroll { x: 7.5, y: 3.5 }, &output);
    assert!(worker.continuation.is_some());
    assert!(worker.set_page(source_page(&source, starts[15], starts[45]), &output));
    assert!(
        (worker.origin[0] - 17.5).abs() < 0.001,
        "crossing discarded horizontal pixels"
    );
}

// The full line and its 38-scalar suffix cover the same eight logical lines.
// At a high source fraction the suffix can falsely satisfy the same scrollbar
// value while still exposing a different first row and a different pixel phase.
fn leading_fragment_source() -> String {
    let mut source = "p\n".repeat(8000);
    source.push_str(&format!("{}{}\n", "P".repeat(38), "A".repeat(61)));
    source.push_str(&format!(
        "b\n{}\nd\n{}\nf\n{}\nh\n",
        "C".repeat(99),
        "E".repeat(99),
        "G".repeat(99)
    ));
    source.push_str(&"z\n".repeat((20_000 - source.len()) / 2));
    assert_eq!(source.len(), 20_000);
    source
}

fn leading_fragment_camera(worker: &Worker) -> (u64, [f32; 2]) {
    leading_fragment_camera_at(worker, 16_038)
}

fn leading_fragment_camera_at(worker: &Worker, source_caret: u64) -> (u64, [f32; 2]) {
    let page = worker.page.as_ref().unwrap();
    let shaped = worker.shaped.as_ref().unwrap();
    let index = shaped
        .hit_test(worker.origin[0], worker.origin[1] + 0.5)
        .unwrap();
    let source = page
        .anchors
        .iter()
        .find(|a| a.text_offset as usize == index)
        .unwrap()
        .source_offset;
    // This exact source caret exists in both the complete row and its suffix.
    let index = page
        .anchors
        .iter()
        .find(|a| a.source_offset == source_caret)
        .unwrap()
        .text_offset as usize;
    let caret = shaped.caret_geometry(index).unwrap();
    (
        source,
        [
            caret.x0 as f32 - worker.origin[0],
            caret.y0 as f32 - worker.origin[1],
        ],
    )
}

fn leading_fragment_reference(source: &str) -> (Worker, Mailbox, ViewerFrame, f64) {
    use super::position::{mapped_prefix_page, mapped_prefix_worker, rendered};
    let (mut reference, output) =
        mapped_prefix_worker(mapped_prefix_page(source, 16_000), source.len() as u64);
    let target = reference
        .scroll_map
        .as_ref()
        .unwrap()
        .metrics(3.5, f64::from(reference.geometry.height))
        .unwrap()
        .0;
    let generation = reference.positions.generation;
    reference.input(
        ViewerInput::Position {
            generation,
            sequence: 4,
            horizontal: false,
            position: target,
        },
        &output,
    );
    let frame = rendered(&mut reference, &output);
    assert_eq!(
        frame.position_sequence,
        [4, 0],
        "the complete-line control must acknowledge the real noninitial camera"
    );
    assert!((reference.origin[1] - 3.5).abs() < 0.001);
    assert_eq!(
        leading_fragment_camera(&reference).0,
        16_000,
        "the complete first row is visible"
    );
    (reference, output, frame, target)
}

fn leading_fragment_pending(source: &str, target: f64) -> (Worker, Mailbox, ViewerFrame) {
    use super::position::{
        mapped_prefix_page, mapped_prefix_worker, recovery_integration_admit, rendered,
    };
    let (mut worker, output) =
        mapped_prefix_worker(mapped_prefix_page(source, 0), source.len() as u64);
    let previous = rendered(&mut worker, &output);
    recovery_integration_admit(&mut worker, 91, 4, target, 16_038, 8);
    let suffix = mapped_prefix_page(source, 16_038);
    assert!(suffix.partial_line);
    assert_eq!(suffix.text.chars().next(), Some('A'));
    worker.event(91, EventKind::TextPage(suffix), &output);
    assert_eq!(worker.positions.acknowledged, previous.position_sequence);
    (worker, output, previous)
}

fn leading_fragment_prefix(worker: &mut Worker, output: &Mailbox) -> messages::Viewport {
    use super::super::position::SourcePosition;
    let (request, candidate) = worker.position_page.take().unwrap();
    assert_eq!(request, 91);
    let end = candidate.end;
    let before = worker.origin;
    // Inspect the resolver's actual reread before transport admission. No plugin
    // session is substituted; the subsequent admitted reply uses real events.
    let SourcePosition::Reload { sequence, viewport } =
        worker.resolve_source_position(candidate, output)
    else {
        panic!(
            "a visible incomplete ordinary first line must request its bounded prefix before publishing or acknowledging"
        );
    };
    assert_eq!(sequence, 4);
    assert!(
        viewport.backwards,
        "the missing context is before the first row"
    );
    assert_eq!(viewport.source_anchor, end);
    assert!(viewport.max_bytes <= 12_288);
    assert!(viewport.max_lines > 8 && viewport.max_lines <= 512);
    assert_eq!(worker.origin, before);
    assert_eq!(worker.positions.acknowledged, [0, 0]);
    assert!(worker.positions.has_pending());
    viewport
}

#[test]
fn source_visible_partial_leading_row_recovers_prefix_before_camera_acknowledgement() {
    use super::position::{mapped_prefix_page, rendered};
    let source = leading_fragment_source();
    let (reference, _, expected, target) = leading_fragment_reference(&source);
    let (mut worker, output, previous) = leading_fragment_pending(&source, target);
    let _viewport = leading_fragment_prefix(&mut worker, &output);
    assert!(worker.page.as_ref().unwrap().text.as_ref() == previous.accessible_text.as_ref());
    worker.positions.requested(92, 4);
    worker.viewport_request = Some(92);
    worker.event(
        92,
        EventKind::TextPage(mapped_prefix_page(&source, 16_000)),
        &output,
    );
    assert_eq!(worker.positions.acknowledged, previous.position_sequence);
    worker.event(92, EventKind::Completed, &output);
    let actual = rendered(&mut worker, &output);
    assert_eq!(actual.position_sequence, [4, 0]);
    assert_eq!(
        leading_fragment_camera(&worker),
        leading_fragment_camera(&reference),
        "the same exact source caret and top row must have the same screen coordinates"
    );
    assert!(
        actual.pixels == expected.pixels,
        "full-line and recovered suffix routes must produce identical visible CPU pixels"
    );
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
}

#[test]
fn source_partial_leading_row_supersession_cannot_acknowledge_a_stale_suffix() {
    use super::position::{mapped_prefix_page, rendered};
    let source = leading_fragment_source();
    let (_, _, _, target) = leading_fragment_reference(&source);
    let (mut worker, output, previous) = leading_fragment_pending(&source, target);
    let newest = worker
        .scroll_map
        .as_ref()
        .unwrap()
        .metrics(7.25, f64::from(worker.geometry.height))
        .unwrap()
        .0;
    worker.input(
        ViewerInput::Position {
            generation: previous.generation,
            sequence: 5,
            horizontal: false,
            position: newest,
        },
        &output,
    );
    assert_eq!(worker.positions.acknowledged, previous.position_sequence);
    worker.event(91, EventKind::Completed, &output);
    let newest_frame = rendered(&mut worker, &output);
    assert_eq!(newest_frame.position_sequence, [5, 0]);
    assert!((worker.origin[1] - 7.25).abs() < 0.001);
    let origin = worker.origin;
    worker.event(
        91,
        EventKind::TextPage(mapped_prefix_page(&source, 16_038)),
        &output,
    );
    worker.event(91, EventKind::Completed, &output);
    assert_eq!(worker.origin, origin);
    assert_eq!(worker.positions.acknowledged, [5, 0]);
    assert!(worker.page.as_ref().unwrap().text.as_ref() == previous.accessible_text.as_ref());
    worker.dirty = true;
    let after = rendered(&mut worker, &output);
    assert!(after.pixels == newest_frame.pixels);
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
}

fn leading_fragment_refresh(
    worker: &mut Worker,
    output: &Mailbox,
    id: u64,
    page: viewer::TextPage,
) {
    worker.text.syntax.demand(
        worker.generation,
        1,
        messages::Viewport {
            source_anchor: 16_000,
            backwards: false,
            max_bytes: 12_288,
            max_lines: 8,
        },
    );
    let key = worker.text.syntax.latest.clone().unwrap();
    worker.text.syntax.started(id, key);
    worker.event(
        id,
        EventKind::SourcePrepared(messages::SourcePrepared {
            revision: 1,
            frontier: 16_000,
            ready: true,
        }),
        output,
    );
    worker.event(id, EventKind::Completed, output);
    worker.text.syntax.refresh_admitted(id + 1);
    worker.event(id + 1, EventKind::TextPage(page), output);
    worker.event(id + 1, EventKind::Completed, output);
}

#[test]
fn source_prepared_partial_leading_row_keeps_exact_camera_until_complete_style_refresh() {
    use super::position::{mapped_prefix_page, rendered};
    let source = leading_fragment_source();
    let (mut worker, output, previous, _) = leading_fragment_reference(&source);
    let before = leading_fragment_camera(&worker);
    let mut suffix = mapped_prefix_page(&source, 16_038);
    suffix.spans.push(viewer::Span {
        start: 0,
        end: suffix.text.len() as u32,
        role: viewer::STYLE_STRING,
        flags: 0,
    });
    leading_fragment_refresh(&mut worker, &output, 101, suffix);
    assert_eq!(leading_fragment_camera(&worker), before);
    assert_eq!(worker.positions.acknowledged, previous.position_sequence);
    worker.dirty = true;
    let pending = rendered(&mut worker, &output);
    assert!(
        pending.pixels == previous.pixels,
        "a style suffix excluding visible source context must retain the coherent CPU viewport"
    );

    let mut full = mapped_prefix_page(&source, 16_000);
    full.spans.push(viewer::Span {
        start: 0,
        end: full.text.len() as u32,
        role: viewer::STYLE_STRING,
        flags: 0,
    });
    leading_fragment_refresh(&mut worker, &output, 103, full.clone());
    let actual = rendered(&mut worker, &output);
    assert_eq!(leading_fragment_camera(&worker), before);
    assert_eq!(actual.position_sequence, previous.position_sequence);
    assert!(
        actual.pixels != previous.pixels,
        "completed syntax must eventually reach the unchanged visible camera"
    );
    let (mut reference, reference_output, _, _) = leading_fragment_reference(&source);
    leading_fragment_refresh(&mut reference, &reference_output, 105, full);
    let expected = rendered(&mut reference, &reference_output);
    assert!(actual.pixels == expected.pixels);
}

fn bounded_partial_source_page(source: &str, start: usize, end: usize) -> viewer::TextPage {
    let text = source[start..end].to_owned();
    let page = viewer::TextPage {
        revision: 1,
        source: viewer::SourceRange {
            offset: start as u64,
            length: (end - start) as u64,
        },
        anchors: text
            .char_indices()
            .map(|(at, _)| viewer::Anchor {
                text_offset: at as u32,
                source_offset: (start + at) as u64,
            })
            .chain(std::iter::once(viewer::Anchor {
                text_offset: text.len() as u32,
                source_offset: end as u64,
            }))
            .collect(),
        spans: Vec::new(),
        has_before: start != 0,
        has_after: end != source.len(),
        partial_line: (start > 0 && !source[..start].ends_with('\n'))
            || (end < source.len() && !text.ends_with('\n')),
        text,
    };
    page.validate().unwrap();
    page
}

#[test]
fn source_covered_interior_byte_fragment_preserves_candidate_and_cached_camera() {
    use super::position::{
        mapped_prefix_page, mapped_prefix_worker, recovery_integration_admit, rendered,
    };
    let mut source = "p\n".repeat(8000);
    source.push_str(&"A".repeat(32_768));
    source.push_str("\ntail\n");
    let (mut worker, output) =
        mapped_prefix_worker(mapped_prefix_page(&source, 0), source.len() as u64);
    let previous = rendered(&mut worker, &output);
    let page = bounded_partial_source_page(&source, 40_000, 40_064);
    assert!(page.partial_line && page.has_before && page.has_after);
    assert!(!page.text.contains('\n'));
    // The short viewport contains all 64 bytes. This hand-authored target owns
    // their first source byte in the remaining file scroll range.
    let target = 40_000.0 / (source.len() - 64) as f64;
    assert!(target > 0.0 && target < 1.0);
    recovery_integration_admit(&mut worker, 91, 4, target, 40_000, 8);
    worker.event(91, EventKind::TextPage(page), &output);
    assert_eq!(worker.positions.acknowledged, previous.position_sequence);
    assert_eq!(worker.page.as_ref().unwrap().start, 0);
    worker.event(91, EventKind::Completed, &output);
    let completed = rendered(&mut worker, &output);
    assert_eq!(completed.position_sequence, [4, 0]);
    assert_eq!(completed.position as f32, target as f32);
    assert_eq!(worker.origin, [0.0; 2]);
    assert!(worker.shaped.as_ref().unwrap().height() < worker.geometry.height as f32);
    assert!(worker.scroll_map.is_none());
    assert_eq!(completed.accessible_text.as_ref(), &source[40_000..40_064]);
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    worker.input(
        ViewerInput::Position {
            generation: completed.generation,
            sequence: 5,
            horizontal: false,
            position: target,
        },
        &output,
    );
    let cached = rendered(&mut worker, &output);
    assert_eq!(cached.position_sequence, [5, 0]);
    assert_eq!(cached.position, completed.position);
    assert_eq!(worker.origin, [0.0; 2]);
    assert!(cached.pixels == completed.pixels);
    assert!(cached.accessible_text == completed.accessible_text);
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
}

#[test]
fn source_enormous_partial_line_exhausts_bounded_prefix_reads_without_acknowledging() {
    use super::super::position::SourcePosition;
    use super::position::{
        mapped_prefix_page, mapped_prefix_worker, recovery_integration_admit, rendered,
    };
    let mut source = "p\n".repeat(8000);
    source.push_str(&"A".repeat(32_768));
    source.push_str("\ntail\n");
    let (mut worker, output) =
        mapped_prefix_worker(mapped_prefix_page(&source, 0), source.len() as u64);
    let previous = rendered(&mut worker, &output);
    let origin = worker.origin;
    let target = 0.81;
    recovery_integration_admit(&mut worker, 91, 4, target, 40_000, 8);
    let end = 40_064;
    let mut start = 40_000;
    worker.event(
        91,
        EventKind::TextPage(bounded_partial_source_page(&source, start, end)),
        &output,
    );
    for (request, expected_lines) in [(92, 16), (93, 32), (94, 64)] {
        let (_, candidate) = worker.position_page.take().unwrap();
        assert!(!candidate.text.contains('\n'));
        let SourcePosition::Reload { sequence, viewport } =
            worker.resolve_source_position(candidate, &output)
        else {
            panic!("missing logical-line context must use the remaining finite prefix allowance");
        };
        assert_eq!(sequence, 4);
        assert!(viewport.backwards);
        assert_eq!(viewport.source_anchor, end as u64);
        assert_eq!(viewport.max_bytes, 12_288);
        assert_eq!(viewport.max_lines, expected_lines);
        assert_eq!(worker.origin, origin);
        assert_eq!(worker.positions.acknowledged, previous.position_sequence);
        // Each bounded provider result contains and strictly grows the previous
        // window, but even the fourth still cannot reach this enormous line's start.
        start -= 64;
        worker.positions.requested(request, 4);
        worker.viewport_request = Some(request);
        worker.event(
            request,
            EventKind::TextPage(bounded_partial_source_page(&source, start, end)),
            &output,
        );
    }
    worker.event(94, EventKind::Completed, &output);
    worker.paint(&output).unwrap();
    let mut unavailable = Vec::new();
    let mut actual = None;
    for event in output.take() {
        match event {
            ViewerOutput::VerticalPositionUnavailable {
                generation,
                sequence,
            } => unavailable.push((generation, sequence)),
            ViewerOutput::Frame(frame) => actual = Some(frame),
            _ => {}
        }
    }
    assert_eq!(unavailable, vec![(previous.generation, 4)]);
    let actual = actual.expect("unavailability retains a real painted viewport");
    assert_eq!(actual.position_sequence, previous.position_sequence);
    assert!(actual.pixels == previous.pixels);
    assert!(actual.accessible_text == previous.accessible_text);
    assert_eq!(worker.origin, origin);
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    // Bounded fragments remain navigable independently of absolute seek refusal.
    let page = Page::text(
        bounded_partial_source_page(&source, start, end),
        &worker.appearance,
    );
    let shaped = worker.layout_page(&page).unwrap();
    let step = crate::viewport::Continuation::capture(
        &page,
        &shaped,
        worker.geometry,
        [0.0; 2],
        17.0,
        true,
    )
    .unwrap();
    assert_eq!(step.anchor, page.end);
}

#[test]
fn source_wrapped_tabbed_partial_first_logical_line_needs_prefix_after_first_visual_row() {
    use super::position::{mapped_prefix_page, mapped_prefix_worker, recovery_integration_admit};
    let mut source = leading_fragment_source();
    source.replace_range(16_038..16_039, "\t");
    let (mut worker, output) =
        mapped_prefix_worker(mapped_prefix_page(&source, 0), source.len() as u64);
    worker.wrap = true;
    worker.geometry.width = 160;
    worker.geometry.height = 40;
    let previous = worker.page.as_ref().unwrap().clone();
    assert!(worker.set_page(previous, &output));
    let suffix = mapped_prefix_page(&source, 16_038);
    assert!(suffix.partial_line && suffix.text.starts_with('\t'));
    let candidate = Page::text(suffix.clone(), &worker.appearance);
    let shaped = worker.layout_page(&candidate).unwrap();
    let rows = shaped.scroll_anchors();
    let origin = rows[1].1 + 3.5;
    let next_logical = candidate.text.find('\n').unwrap() + 1;
    let next_logical_y = shaped.caret_geometry(next_logical).unwrap().y0 as f32;
    assert!(
        origin > rows[1].1 && origin < next_logical_y,
        "the first visual row is gone while its incomplete logical line remains visible"
    );
    let map = super::super::scroll_map::build(
        &candidate,
        &shaped,
        worker.length,
        true,
        worker.geometry.height,
    )
    .unwrap();
    let target = map
        .metrics(f64::from(origin), f64::from(worker.geometry.height))
        .unwrap()
        .0;
    recovery_integration_admit(&mut worker, 91, 4, target, 16_038, 8);
    worker.event(91, EventKind::TextPage(suffix), &output);
    let _viewport = leading_fragment_prefix(&mut worker, &output);
}

#[test]
fn source_bof_with_a_partial_trailing_line_can_acknowledge_its_complete_first_row() {
    use super::super::position::SourcePosition;
    use super::position::{mapped_prefix_page, mapped_prefix_worker, recovery_integration_admit};
    let source = leading_fragment_source();
    let (mut worker, output) =
        mapped_prefix_worker(mapped_prefix_page(&source, 0), source.len() as u64);
    let page = bounded_partial_source_page(&source, 0, 17);
    assert!(!page.has_before && page.partial_line);
    let candidate = Page::text(page.clone(), &worker.appearance);
    let shaped = worker.layout_page(&candidate).unwrap();
    let map = super::super::scroll_map::build(
        &candidate,
        &shaped,
        worker.length,
        true,
        worker.geometry.height,
    )
    .unwrap();
    let target = map
        .metrics(3.5, f64::from(worker.geometry.height))
        .unwrap()
        .0;
    recovery_integration_admit(&mut worker, 91, 4, target, 0, 8);
    worker.event(91, EventKind::TextPage(page), &output);
    let (_, candidate) = worker.position_page.take().unwrap();
    assert!(
        matches!(
            worker.resolve_source_position(candidate, &output),
            SourcePosition::Ready
        ),
        "BOF proves leading context even when the opposite edge is partial"
    );
    assert!((worker.origin[1] - 3.5).abs() < 0.001);
    assert_eq!(
        worker.positions.complete(91, true),
        crate::position_ack::Completion::Ready
    );
    assert_eq!(worker.positions.acknowledged, [4, 0]);
}

#[test]
fn source_partial_leading_row_recovery_uses_exact_utf16_source_anchors() {
    use super::position::{mapped_prefix_worker, recovery_integration_admit, rendered};
    let mut source = leading_fragment_source();
    source.replace_range(16_038..16_044, "東京");
    let utf16_page = |mut page: viewer::TextPage| {
        let start = page.source.offset as usize;
        let end = page.source.end().unwrap() as usize;
        page.source.offset = source[..start].encode_utf16().count() as u64 * 2;
        page.source.length = source[start..end].encode_utf16().count() as u64 * 2;
        for anchor in &mut page.anchors {
            anchor.source_offset = source[..anchor.source_offset as usize]
                .encode_utf16()
                .count() as u64
                * 2;
        }
        page.validate().unwrap();
        page
    };
    let length = source.encode_utf16().count() as u64 * 2;
    let full = utf16_page(bounded_partial_source_page(&source, 16_000, 16_408));
    let suffix = utf16_page(bounded_partial_source_page(&source, 16_038, 16_408));
    assert_eq!(suffix.source.offset, 32_076);
    assert!(
        suffix
            .anchors
            .iter()
            .any(|a| a.text_offset == 3 && a.source_offset == 32_078),
        "one three-byte UTF-8 scalar advances two actual UTF-16 source bytes"
    );
    let (mut reference, reference_output) = mapped_prefix_worker(full.clone(), length);
    let target = reference
        .scroll_map
        .as_ref()
        .unwrap()
        .metrics(3.5, f64::from(reference.geometry.height))
        .unwrap()
        .0;
    reference.input(
        ViewerInput::Position {
            generation: reference.positions.generation,
            sequence: 4,
            horizontal: false,
            position: target,
        },
        &reference_output,
    );
    let expected = rendered(&mut reference, &reference_output);
    assert_eq!(expected.position_sequence, [4, 0]);
    assert_eq!(leading_fragment_camera_at(&reference, 32_076).0, 32_000);

    let initial = utf16_page(bounded_partial_source_page(&source, 0, 16));
    let (mut worker, output) = mapped_prefix_worker(initial, length);
    // The reference includes fallback glyphs in its first six rows; the ASCII
    // initial page does not. Derive one camera from the reference and apply it
    // through the real resize/reflow path before comparing exact source pixels.
    worker.input(ViewerInput::Resize(reference.geometry), &output);
    worker.reflow(&output);
    assert_eq!(
        (
            worker.geometry.width,
            worker.geometry.height,
            worker.geometry.scale
        ),
        (
            reference.geometry.width,
            reference.geometry.height,
            reference.geometry.scale
        )
    );
    let _previous = rendered(&mut worker, &output);
    recovery_integration_admit(&mut worker, 91, 4, target, 32_076, 8);
    worker.event(91, EventKind::TextPage(suffix), &output);
    let _viewport = leading_fragment_prefix(&mut worker, &output);
    worker.positions.requested(92, 4);
    worker.viewport_request = Some(92);
    worker.event(92, EventKind::TextPage(full), &output);
    worker.event(92, EventKind::Completed, &output);
    let actual = rendered(&mut worker, &output);
    assert_eq!(actual.position_sequence, [4, 0]);
    assert_eq!(
        leading_fragment_camera_at(&worker, 32_076),
        leading_fragment_camera_at(&reference, 32_076)
    );
    assert!(actual.pixels == expected.pixels);
}

#[test]
fn source_trailing_only_partial_page_keeps_complete_visible_logical_line_usable() {
    use super::position::{
        mapped_prefix_page, mapped_prefix_worker, recovery_integration_admit, rendered,
    };
    let source = leading_fragment_source();
    let (reference, _, expected, target) = leading_fragment_reference(&source);
    let (mut worker, output) =
        mapped_prefix_worker(mapped_prefix_page(&source, 0), source.len() as u64);
    let _previous = rendered(&mut worker, &output);
    // Only the last 'h' row is cut. The extra preceding p row makes the complete
    // visible first large line follow an actual decoded logical break, despite
    // partial_line conservatively describing either end of the packet.
    let page = bounded_partial_source_page(&source, 15_998, 16_407);
    assert!(page.has_before && page.has_after && page.partial_line);
    assert!(page.text.starts_with("p\nPP") && page.text.ends_with('h'));
    recovery_integration_admit(&mut worker, 91, 4, target, 15_998, 8);
    worker.event(91, EventKind::TextPage(page), &output);
    worker.event(91, EventKind::Completed, &output);
    let actual = rendered(&mut worker, &output);
    assert_eq!(actual.position_sequence, [4, 0]);
    let (actual_source, actual_caret) = leading_fragment_camera(&worker);
    let (expected_source, expected_caret) = leading_fragment_camera(&reference);
    assert_eq!(actual_source, expected_source);
    for (actual, expected) in actual_caret.into_iter().zip(expected_caret) {
        assert!(
            (actual - expected).abs() < 0.001,
            "exact source caret moved on screen: {actual} versus {expected}"
        );
    }
    assert!(actual.pixels == expected.pixels);
}

#[test]
fn source_real_terminal_byte_fragment_keeps_candidate_and_cached_end_authority() {
    use super::position::{
        mapped_prefix_page, mapped_prefix_worker, recovery_integration_admit, rendered,
    };
    let source = format!("{}{}", "p\n".repeat(8000), "A".repeat(4000));
    let (mut worker, output) =
        mapped_prefix_worker(mapped_prefix_page(&source, 0), source.len() as u64);
    let previous = rendered(&mut worker, &output);
    let suffix = bounded_partial_source_page(&source, 19_936, 20_000);
    assert!(suffix.partial_line && !suffix.has_after && !suffix.text.contains('\n'));
    recovery_integration_admit(&mut worker, 91, 4, 1.0, 20_000, 8);
    worker.event(91, EventKind::TextPage(suffix), &output);
    assert_eq!(worker.positions.acknowledged, previous.position_sequence);
    worker.event(91, EventKind::Completed, &output);
    let endpoint = rendered(&mut worker, &output);
    assert_eq!(endpoint.position_sequence, [4, 0]);
    assert_eq!(endpoint.position, 1.0);
    assert_eq!(worker.origin, [0.0; 2]);
    assert!(worker.scroll_map.is_none());
    assert_eq!(endpoint.accessible_text.as_ref(), &source[19_936..]);
    assert!(endpoint.pixels != previous.pixels);
    worker.input(
        ViewerInput::Position {
            generation: endpoint.generation,
            sequence: 5,
            horizontal: false,
            position: 1.0,
        },
        &output,
    );
    let cached = rendered(&mut worker, &output);
    assert_eq!(cached.position_sequence, [5, 0]);
    assert_eq!(cached.position, 1.0);
    assert!(cached.pixels == endpoint.pixels);
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
}

#[test]
fn source_terminal_byte_exception_rejects_clipped_edges_and_interior_near_end() {
    use super::position::{
        mapped_prefix_page, mapped_prefix_worker, recovery_integration_admit, rendered,
    };
    let source = format!("{}{}", "p\n".repeat(8000), "A".repeat(4000));
    for (target, start, end, before, after) in [
        (0.0, 2, 3, false, true),
        (1.0, 19_935, 19_999, true, false),
        (1.0, 19_936, 20_000, true, true),
        (0.999, 19_936, 20_000, true, false),
    ] {
        let (mut worker, output) =
            mapped_prefix_worker(mapped_prefix_page(&source, 0), source.len() as u64);
        let previous = rendered(&mut worker, &output);
        let mut suffix = bounded_partial_source_page(&source, start, end);
        suffix.has_before = before;
        suffix.has_after = after;
        assert!(suffix.partial_line && !suffix.text.contains('\n'));
        recovery_integration_admit(&mut worker, 91, 4, target, end as u64, 8);
        worker.event(91, EventKind::TextPage(suffix), &output);
        worker.event(91, EventKind::Completed, &output);
        worker.paint(&output).unwrap();
        let mut unavailable = Vec::new();
        let mut frame = None;
        for event in output.take() {
            match event {
                ViewerOutput::VerticalPositionUnavailable {
                    generation,
                    sequence,
                } => unavailable.push((generation, sequence)),
                ViewerOutput::Frame(current) => frame = Some(current),
                _ => {}
            }
        }
        assert_eq!(
            unavailable,
            vec![(previous.generation, 4)],
            "target={target}, end={end}, after={after}"
        );
        let frame = frame.expect("failed endpoint preserves a painted camera");
        assert_eq!(frame.position_sequence, previous.position_sequence);
        assert!(frame.pixels == previous.pixels);
        assert!(frame.accessible_text == previous.accessible_text);
        assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    }
}
