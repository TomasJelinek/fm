use super::*;

fn prepared(mode: u16, retained: bool, partial: bool) -> (Worker, Mailbox) {
    let (mut worker, output) = worker();
    worker.mode = mode;
    worker.geometry = ViewerGeometry {
        width: 350,
        height: 110,
        scale: 1.0,
    };
    let source: String = (0..80)
        .map(|row| format!("line {row}: {}\n", "word ".repeat(row % 7 + 1)))
        .collect();
    let mut decoded = decoded_page(&source);
    decoded.source.offset = if partial { 4000 } else { 0 };
    decoded.has_before = partial;
    decoded.has_after = partial;
    decoded.anchors = source
        .char_indices()
        .map(|(at, _)| viewer::Anchor {
            text_offset: at as u32,
            source_offset: decoded.source.offset + at as u64,
        })
        .chain(std::iter::once(viewer::Anchor {
            text_offset: source.len() as u32,
            source_offset: decoded.source.offset + source.len() as u64,
        }))
        .collect();
    worker.length = if partial { 20_000 } else { source.len() as u64 };
    worker.input(ViewerInput::RetainedRaster(retained), &output);
    assert!(worker.set_page(Page::text(decoded, &worker.appearance), &output));
    (worker, output)
}

fn rendered(worker: &mut Worker, output: &Mailbox) -> ViewerFrame {
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
fn scrollbar_position_complete_text_and_source_use_visual_range_without_another_page() {
    for mode in [viewer::MODE_TEXT, viewer::MODE_SOURCE] {
        for retained in [false, true] {
            let (mut worker, output) = prepared(mode, retained, false);
            let generation = worker.positions.generation;
            let range = worker.shaped.as_ref().unwrap().height() - worker.geometry.height as f32;
            for (index, position) in [0.7, 1.0, 0.3125, 0.0].into_iter().enumerate() {
                worker.input(
                    ViewerInput::Position {
                        generation,
                        sequence: index as u64 + 1,
                        horizontal: false,
                        position,
                    },
                    &output,
                );
                let frame = rendered(&mut worker, &output);
                assert!(
                    (frame.position - position).abs() < 0.0001,
                    "{mode}, retained={retained}: requested {position}, got {}",
                    frame.position
                );
                assert!((worker.origin[1] - position as f32 * range).abs() < 0.001);
                assert_eq!(frame.generation, generation);
                assert_eq!(frame.position_sequence[0], index as u64 + 1);
                assert!(worker.viewport_request.is_none());
                assert!(
                    !worker.page.as_ref().unwrap().before && !worker.page.as_ref().unwrap().after
                );
            }
        }
    }
}

#[test]
fn scrollbar_position_inside_a_partial_decoded_page_preserves_fractional_position() {
    let (mut worker, output) = prepared(viewer::MODE_SOURCE, true, true);
    let target_y = 73.25;
    let position = position_at(&worker, target_y);
    let generation = worker.positions.generation;
    worker.input(
        ViewerInput::Position {
            generation,
            sequence: 1,
            horizontal: false,
            position,
        },
        &output,
    );
    let frame = rendered(&mut worker, &output);
    assert!((worker.origin[1] - target_y).abs() < 0.001);
    assert!((frame.position - position).abs() < 0.0001);
    assert_eq!(frame.position_sequence[0], 1);
    assert!(worker.page.as_ref().unwrap().before && worker.page.as_ref().unwrap().after);
}

#[test]
fn scrollbar_position_does_not_accept_a_stale_sequence_or_false_eof() {
    let (mut worker, output) = prepared(viewer::MODE_TEXT, true, false);
    let generation = worker.positions.generation;
    worker.input(
        ViewerInput::Position {
            generation,
            sequence: 5,
            horizontal: false,
            position: 0.7,
        },
        &output,
    );
    let frame = rendered(&mut worker, &output);
    assert!((frame.position - 0.7).abs() < 0.0001);
    worker.input(
        ViewerInput::Position {
            generation,
            sequence: 4,
            horizontal: false,
            position: 1.0,
        },
        &output,
    );
    let frame = rendered(&mut worker, &output);
    assert!((frame.position - 0.7).abs() < 0.0001);
    assert_eq!(frame.position_sequence[0], 5);
}

#[test]
fn scrollbar_position_byte_estimate_excludes_the_visible_extent() {
    assert_eq!(
        navigation::scrollbar_viewport(0.7, 1000, 0.25),
        (525, false)
    );
    assert_eq!(
        navigation::scrollbar_viewport(1.0, 1000, 0.25),
        (1000, true)
    );
    assert_eq!(navigation::scrollbar_viewport(0.0, 1000, 0.25), (0, false));
    assert_eq!(
        navigation::scrollbar_viewport(f64::NAN, 1000, 0.25),
        (0, false)
    );
    assert_eq!(navigation::scrollbar_viewport(0.7, 0, 1.0), (0, false));
    assert!(navigation::scrollbar_viewport(0.7, 10_000_000_000, 0.01).0 > u64::from(u32::MAX));
}

#[test]
fn scrollbar_position_small_page_line_budget_does_not_expand_large_file_reads() {
    let geometry = ViewerGeometry {
        width: 100,
        height: 100,
        scale: 1.0,
    };
    for mode in [viewer::MODE_TEXT, viewer::MODE_SOURCE] {
        assert_eq!(
            crate::viewport::request_line_budget(geometry, 20.0, 8192, mode),
            512
        );
        for length in [8193, 10_000_000_000, u64::MAX] {
            assert_eq!(
                crate::viewport::request_line_budget(geometry, 20.0, length, mode),
                7
            );
        }
    }
    assert_eq!(
        crate::viewport::request_line_budget(geometry, 20.0, 4000, viewer::MODE_BINARY),
        7
    );
}

#[test]
fn scrollbar_position_deferred_seek_keeps_the_latest_origin_after_read_completion() {
    let (mut worker, output) = prepared(viewer::MODE_SOURCE, true, false);
    let generation = worker.positions.generation;
    assert!(worker.positions.accept(generation, 1, false, 0.1));
    worker.positions.requested(91, 1);
    worker.positions.loaded(91);
    worker.viewport_request = Some(91);
    worker.position_page = Some((91, worker.page.as_ref().unwrap().clone()));
    worker.input(
        ViewerInput::Position {
            generation,
            sequence: 2,
            horizontal: false,
            position: 0.7,
        },
        &output,
    );
    worker.event(91, EventKind::Completed, &output);
    let frame = rendered(&mut worker, &output);
    assert_eq!(frame.position_sequence[0], 2);
    assert!(
        (frame.position - 0.7).abs() < 0.0001,
        "deferred seek was pulled back to {}",
        frame.position
    );
}

fn read_target(worker: &mut Worker, request: u64, sequence: u64, position: f64) {
    worker.positions.target = Some(crate::position_ack::Target {
        sequence,
        position,
        corrected: false,
    });
    worker.positions.requested(request, sequence);
    worker.positions.loaded(request);
    worker.viewport_request = Some(request);
    worker.position_page = Some((request, worker.page.as_ref().unwrap().clone()));
}

fn position_at(worker: &Worker, y: f32) -> f64 {
    worker
        .scroll_map
        .as_ref()
        .expect("prepared source anchors")
        .metrics(f64::from(y), f64::from(worker.geometry.height))
        .unwrap()
        .0
}

#[test]
fn scrollbar_loaded_partial_page_resolves_exact_target_before_acknowledging() {
    for mode in [viewer::MODE_SOURCE, viewer::MODE_DOCUMENT] {
        let (mut worker, output) = prepared(mode, true, true);
        let position = position_at(&worker, 73.25);
        assert!((position_at(&worker, 0.0) - position).abs() > 0.0001);
        read_target(&mut worker, 91, 1, position);
        worker.event(91, EventKind::Completed, &output);
        let frame = rendered(&mut worker, &output);
        assert_eq!(frame.position_sequence[0], 1);
        assert!(
            (frame.position - position).abs() < 0.0001,
            "new fragment acknowledged the byte estimate rather than the requested camera: mode={mode}, target={position}, actual={}",
            frame.position
        );
        assert!((worker.origin[1] - 73.25).abs() < 0.001);
    }
}

#[test]
fn scrollbar_density_shift_uses_one_guarded_correction_then_actual_pixels_at_the_target() {
    let (mut worker, output) = prepared(viewer::MODE_SOURCE, true, true);
    let at_start = position_at(&worker, 0.0);
    let extent = navigation::scroll_metrics(
        worker.page.as_ref().unwrap().start,
        worker.page.as_ref().unwrap().end,
        worker.length,
        worker.shaped.as_ref().unwrap().height(),
        worker.geometry.height as f32,
        0.0,
    )
    .1;
    let target = at_start - 80.0 / (worker.length as f64 * (1.0 - extent));
    read_target(&mut worker, 91, 1, target);
    let super::super::position::LoadedPosition::Reload {
        sequence,
        anchor,
        backwards,
    } = worker.refine_position()
    else {
        panic!("new density puts the desired camera before the loaded fragment");
    };
    assert_eq!(sequence, 1);
    assert!(!backwards);
    let mut page = worker.page.as_ref().unwrap().clone();
    assert!(
        anchor < page.start - 80,
        "correction must retain prefix context"
    );
    assert!(
        page.start - anchor <= page.end - page.start,
        "context stays within one bounded source page"
    );
    let shift = page.start - anchor;
    page.start -= shift;
    page.end -= shift;
    for mapped in &mut page.anchors {
        mapped.source_offset -= shift;
    }
    worker.positions.requested(92, sequence);
    worker.positions.loaded(92);
    worker.viewport_request = Some(92);
    worker.position_page = Some((92, page));
    worker.event(92, EventKind::Completed, &output);
    let frame = rendered(&mut worker, &output);
    assert_eq!(frame.position_sequence[0], 1);
    assert!((frame.position - target).abs() < 0.0001);
    assert!(worker.origin[1] > 0.0);
    assert!(worker.viewport_request.is_none());
}

#[test]
fn scrollbar_uncovered_correction_fails_without_false_ack_or_another_read() {
    let (mut worker, output) = prepared(viewer::MODE_SOURCE, true, true);
    read_target(&mut worker, 91, 1, 0.9);
    assert!(matches!(
        worker.refine_position(),
        super::super::position::LoadedPosition::Reload { .. }
    ));
    assert_eq!(
        worker.refine_position(),
        super::super::position::LoadedPosition::Unavailable
    );
    worker.event(91, EventKind::Completed, &output);
    assert_eq!(worker.positions.acknowledged[0], 0);
    assert!(worker.viewport_request.is_none());
    assert!(worker.status.contains("not completed"));
}

#[test]
fn scrollbar_latest_deferred_target_supersedes_an_uncovered_old_correction() {
    let (mut worker, output) = prepared(viewer::MODE_SOURCE, true, true);
    let generation = worker.positions.generation;
    read_target(&mut worker, 91, 1, 0.9);
    worker.positions.target.as_mut().unwrap().corrected = true;
    let target = position_at(&worker, 113.75);
    worker.input(
        ViewerInput::Position {
            generation,
            sequence: 2,
            horizontal: false,
            position: target,
        },
        &output,
    );
    worker.event(91, EventKind::Completed, &output);
    let frame = rendered(&mut worker, &output);
    assert_eq!(frame.position_sequence[0], 2);
    assert_eq!(frame.generation, generation);
    assert!((frame.position - target).abs() < 0.0001);
    assert!(worker.viewport_request.is_none());
}

#[test]
fn scrollbar_correction_prefix_fits_a_fragment_with_only_twenty_pixels_to_scroll() {
    for retained in [false, true] {
        let (mut worker, output) = prepared(viewer::MODE_SOURCE, retained, true);
        let available = crate::viewport::available_height(
            worker.page.as_ref().unwrap(),
            worker.shaped.as_ref().unwrap(),
            worker.geometry.height as f32,
        );
        worker.geometry.height = available as u32 - 20;
        let (position, extent) = navigation::scroll_metrics(
            worker.page.as_ref().unwrap().start,
            worker.page.as_ref().unwrap().end,
            worker.length,
            worker.shaped.as_ref().unwrap().height(),
            worker.geometry.height as f32,
            0.0,
        );
        let target = position - 20.0 / (worker.length as f64 * (1.0 - extent));
        read_target(&mut worker, 91, 1, target);
        let super::super::position::LoadedPosition::Reload { anchor, .. } =
            worker.refine_position()
        else {
            panic!("target needs a preceding source fragment");
        };
        let mut page = worker.page.as_ref().unwrap().clone();
        let shift = page.start - anchor;
        page.start -= shift;
        page.end -= shift;
        for mapped in &mut page.anchors {
            mapped.source_offset -= shift;
        }
        worker.position_page = Some((91, page));
        worker.event(91, EventKind::Completed, &output);
        let frame = rendered(&mut worker, &output);
        assert_eq!(frame.position_sequence[0], 1, "retained={retained}");
        assert!((frame.position - target).abs() < 0.0001);
    }
}

#[test]
fn scrollbar_short_byte_limited_fragment_keeps_its_current_position_without_a_read() {
    let (mut worker, output) = worker();
    let mut decoded = decoded_page("bounded line");
    decoded.source.offset = 4000;
    decoded.has_before = true;
    decoded.has_after = true;
    for anchor in &mut decoded.anchors {
        anchor.source_offset += 4000;
    }
    worker.length = 40_000;
    assert!(worker.set_page(Page::text(decoded, &worker.appearance), &output));
    assert!(worker.shaped.as_ref().unwrap().height() < worker.geometry.height as f32);
    let first = rendered(&mut worker, &output);
    worker.input(
        ViewerInput::Position {
            generation: first.generation,
            sequence: 1,
            horizontal: false,
            position: first.position,
        },
        &output,
    );
    let next = rendered(&mut worker, &output);
    assert_eq!(next.position_sequence[0], 1);
    assert_eq!(next.position, first.position);
    assert!(worker.viewport_request.is_none());
}
