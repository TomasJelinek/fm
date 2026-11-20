//! Bounded Text viewport correction at an incomplete file edge.
use super::super::position::LoadedPosition;
use super::position::rendered;
use super::*;

const LENGTH: usize = 92_746;
const TAIL_START: usize = 92_140;

fn edge_source(after: bool) -> String {
    // Independent ASCII rows preserve the measured native failure's density
    // without copying the Lua corpus. Lengths include their terminating LF.
    let mut lengths = [13, 123, 123, 4, 34, 20, 42, 47, 42, 12, 8, 67, 4, 67];
    if !after {
        lengths.reverse();
    }
    let tail: String = lengths
        .into_iter()
        .enumerate()
        .map(|(row, length)| {
            format!(
                "{}\n",
                char::from(b'A' + row as u8).to_string().repeat(length - 1)
            )
        })
        .collect();
    assert_eq!(tail.len(), 606);
    let prefix = "prefix row 01234567\n".repeat(4607);
    assert_eq!(prefix.len(), TAIL_START);
    let source = if after {
        format!("{prefix}{tail}")
    } else {
        format!("{tail}{prefix}")
    };
    assert_eq!(source.len(), LENGTH);
    source
}

pub(super) fn edge_page(source: &str, start: usize, end: usize) -> viewer::TextPage {
    let text = &source[start..end];
    let page = viewer::TextPage {
        revision: 1,
        source: viewer::SourceRange {
            offset: start as u64,
            length: (end - start) as u64,
        },
        text: text.into(),
        spans: vec![],
        anchors: (0..=text.len())
            .map(|at| viewer::Anchor {
                text_offset: at as u32,
                source_offset: (start + at) as u64,
            })
            .collect(),
        has_before: start > 0,
        has_after: end < source.len(),
        partial_line: start > 0 && !source[..start].ends_with('\n'),
    };
    midnight_plugin_host::api::codec::Model::validate(&page).unwrap();
    page
}

pub(super) fn edge_worker(page: viewer::TextPage, length: u64) -> (Worker, Mailbox) {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_TEXT;
    worker.length = length;
    worker.geometry = ViewerGeometry {
        width: 900,
        height: 120,
        scale: 1.0,
    };
    worker.raster.enabled = false;
    worker.wrap = false;
    worker.positions.reset().unwrap();
    assert!(worker.set_page(Page::text(page, &worker.appearance), &output));
    (worker, output)
}

fn edge_pending(after: bool) -> (Worker, Mailbox, String, ViewerFrame, f64) {
    let source = edge_source(after);
    let (start, end, target) = if after {
        (92_140, 92_600, 0.999)
    } else {
        (146, 606, 0.001)
    };
    let page = edge_page(&source, start, end);
    assert_eq!(page.text.bytes().filter(|&b| b == b'\n').count(), 10);
    let (mut worker, output) = edge_worker(page, LENGTH as u64);
    assert!(
        worker.scroll_map.is_some(),
        "real scalar anchors must produce the native row map"
    );
    let first = rendered(&mut worker, &output);
    let page = worker.page.as_ref().unwrap();
    let shaped = worker.shaped.as_ref().unwrap();
    let maximum = (crate::viewport::available_height(page, shaped, 120.0) - 120.0).max(0.0);
    let map = worker.scroll_map.as_ref().unwrap();
    let low = map.metrics(0.0, 120.0).unwrap().0;
    let high = map.metrics(f64::from(maximum), 120.0).unwrap().0;
    assert!(
        if after { target > high } else { target < low },
        "the real shaped row map must miss this camera: after={after}, low={low}, high={high}"
    );
    assert!(worker.positions.accept(first.generation, 4, false, target));
    worker.positions.target = Some(crate::position_ack::Target {
        sequence: 4,
        position: target,
        corrected: false,
    });
    worker.positions.requested(91, 4);
    worker.positions.loaded(91);
    worker.viewport_request = Some(91);
    worker.position_page = Some((91, worker.page.as_ref().unwrap().clone()));
    (worker, output, source, first, target)
}

fn edge_correction(worker: &mut Worker, after: bool) -> messages::Viewport {
    let LoadedPosition::ReloadViewport { sequence, viewport } = worker.refine_position() else {
        panic!("an incomplete Text camera needs one bounded corrective viewport");
    };
    assert_eq!(sequence, 4);
    assert_eq!(
        viewport.source_anchor,
        if after { 92_140 } else { 606 },
        "a Text edge correction must retain the existing window instead of shifting it"
    );
    assert_eq!(viewport.backwards, !after);
    assert_eq!(
        viewport.max_lines, 20,
        "grow the ten-line budget within the same byte bound"
    );
    assert_eq!(viewport.max_bytes, 12_288);
    viewport
}

#[test]
fn text_edge_correction_grows_the_existing_window_before_acknowledging_actual_pixels() {
    for after in [true, false] {
        let (mut worker, output, source, first, target) = edge_pending(after);
        let _viewport = edge_correction(&mut worker, after);
        assert_eq!(worker.positions.acknowledged, first.position_sequence);
        let (start, end) = if after {
            (TAIL_START, LENGTH)
        } else {
            (0, 606)
        };
        let complete = edge_page(&source, start, end);
        assert_eq!(complete.text.bytes().filter(|&b| b == b'\n').count(), 14);
        // A separately prepared full edge is the real renderer/pixel control.
        let (mut reference, reference_output) = edge_worker(complete.clone(), LENGTH as u64);
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
        assert_eq!(
            expected.position_sequence,
            [4, 0],
            "the full edge must cover the target"
        );
        worker.positions.requested(92, 4);
        worker.viewport_request = Some(92);
        worker.event(92, EventKind::TextPage(complete), &output);
        assert_eq!(worker.positions.acknowledged, first.position_sequence);
        assert!(worker.page.as_ref().unwrap().text.as_ref() == first.accessible_text.as_ref());
        worker.event(92, EventKind::Completed, &output);
        let actual = rendered(&mut worker, &output);
        assert_eq!(
            actual.generation, first.generation,
            "a covered Text seek must retain its generation"
        );
        assert_eq!(actual.position_sequence, [4, 0]);
        assert_eq!(actual.position, expected.position);
        assert_eq!(worker.origin, reference.origin);
        assert!(actual.pixels == expected.pixels);
        assert!(actual.accessible_text == expected.accessible_text);
        assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
        assert!(worker.status.is_empty());
    }
}

/// The frame painted after a terminal event, and whether the position was refused.
pub(super) fn settled_frame(worker: &mut Worker, output: &Mailbox) -> (ViewerFrame, bool) {
    worker.paint(output).unwrap();
    let mut frame = None;
    let mut unavailable = false;
    for message in output.take() {
        match message {
            ViewerOutput::Frame(current) => frame = Some(*current),
            ViewerOutput::VerticalPositionUnavailable { .. } => unavailable = true,
            _ => {}
        }
    }
    (frame.expect("a painted frame"), unavailable)
}

#[test]
fn text_edge_uncovered_corrected_reply_settles_at_the_nearest_edge_without_a_third_read() {
    let (mut worker, output, source, first, target) = edge_pending(true);
    let _viewport = edge_correction(&mut worker, true);
    worker.positions.requested(92, 4);
    worker.viewport_request = Some(92);
    unsent::take();
    worker.event(
        92,
        EventKind::TextPage(edge_page(&source, 92_140, 92_600)),
        &output,
    );
    assert_eq!(worker.positions.acknowledged, first.position_sequence);
    worker.event(92, EventKind::Completed, &output);
    let (settled, unavailable) = settled_frame(&mut worker, &output);
    assert!(
        unsent::take().is_empty(),
        "a still-uncovered camera cannot reserve a third read"
    );
    // The loaded window's bottom is the closest view of the uncovered target.
    let page = worker.page.as_ref().unwrap();
    let shaped = worker.shaped.as_ref().unwrap();
    let maximum = (crate::viewport::available_height(page, shaped, 120.0) - 120.0).max(0.0);
    assert_eq!(worker.origin[1], maximum);
    assert!(
        !unavailable && worker.status.is_empty(),
        "{}",
        worker.status
    );
    assert_eq!(settled.position_sequence, [4, 0]);
    assert_eq!(settled.generation, first.generation);
    assert!(
        settled.position < target,
        "the frame reports the view it shows: {}",
        settled.position
    );
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
}

#[test]
fn text_edge_byte_limited_and_maximum_line_windows_keep_the_legacy_correction() {
    let large = edge_source(true);
    let small = format!(
        "{}{}",
        "prefix row 01234567\n".repeat(100),
        &large[TAIL_START..]
    );
    for (source, start, end, mapped) in [(large, 92_153, 92_217, false), (small, 2000, 2460, true)]
    {
        let page = edge_page(&source, start, end);
        let (mut worker, output) = edge_worker(page, source.len() as u64);
        let first = rendered(&mut worker, &output);
        assert_eq!(worker.scroll_map.is_some(), mapped);
        if mapped {
            assert_eq!(worker.page_viewport(start as u64, false).max_lines, 512);
        } else {
            assert!(worker.shaped.as_ref().unwrap().height() < worker.geometry.height as f32);
        }
        assert!(worker.positions.accept(first.generation, 4, false, 0.999));
        worker.positions.target = Some(crate::position_ack::Target {
            sequence: 4,
            position: 0.999,
            corrected: false,
        });
        worker.positions.requested(91, 4);
        worker.positions.loaded(91);
        worker.viewport_request = Some(91);
        worker.position_page = Some((91, worker.page.as_ref().unwrap().clone()));
        let LoadedPosition::Reload {
            sequence,
            anchor,
            backwards,
        } = worker.refine_position()
        else {
            panic!("byte-limited or maximum-line packets must retain their useful legacy shift");
        };
        assert_eq!(sequence, 4);
        assert!(!backwards && anchor != start as u64);
        let end = if mapped {
            source.len()
        } else {
            anchor as usize + 64
        };
        worker.positions.requested(92, 4);
        worker.viewport_request = Some(92);
        worker.event(
            92,
            EventKind::TextPage(edge_page(&source, anchor as usize, end)),
            &output,
        );
        worker.event(92, EventKind::Completed, &output);
        let completed = rendered(&mut worker, &output);
        assert_eq!(completed.generation, first.generation);
        assert_eq!(completed.position_sequence, [4, 0]);
        assert!((completed.position - 0.999).abs() < 0.0001);
        assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    }
}
