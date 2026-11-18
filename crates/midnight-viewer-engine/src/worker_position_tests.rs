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

fn mapped_prefix_source(dense_first: bool) -> String {
    let dense: String = ['A', 'B', 'C']
        .into_iter()
        .map(|letter| format!("{}\n", letter.to_string().repeat(99)))
        .collect();
    let body = if dense_first {
        format!("{dense}{}", "y\n".repeat(5))
    } else {
        format!("{}{dense}{}", "x\n".repeat(3), "y\n".repeat(2))
    };
    assert_eq!(body.len(), 310);
    assert_eq!(body.bytes().filter(|&byte| byte == b'\n').count(), 8);
    let mut source = "p\n".repeat(2000);
    source.push_str(&body);
    source.push_str(&"z\n".repeat((20_000 - source.len()) / 2));
    assert_eq!(source.len(), 20_000);
    source
}

/// An authored ASCII Source response: eight included LFs, scalar anchors and
/// the real source slice. The plugin's foreground byte budget cannot crop it.
fn mapped_prefix_page(source: &str, anchor: u64) -> viewer::TextPage {
    assert!(source.is_ascii());
    let start = usize::try_from(anchor).unwrap();
    let text: String = source[start..].split_inclusive('\n').take(8).collect();
    assert!(text.len() <= 8192);
    let end = start + text.len();
    let page = viewer::TextPage {
        revision: 1,
        source: viewer::SourceRange {
            offset: anchor,
            length: text.len() as u64,
        },
        anchors: text
            .char_indices()
            .map(|(at, _)| viewer::Anchor {
                text_offset: at as u32,
                source_offset: anchor + at as u64,
            })
            .chain(std::iter::once(viewer::Anchor {
                text_offset: text.len() as u32,
                source_offset: anchor + text.len() as u64,
            }))
            .collect(),
        partial_line: (start > 0 && !source[..start].ends_with('\n'))
            || (end < source.len() && !text.ends_with('\n')),
        text,
        spans: Vec::new(),
        has_before: start > 0,
        has_after: end < source.len(),
    };
    midnight_plugin_host::api::codec::Model::validate(&page).unwrap();
    page
}

fn mapped_prefix_worker(page: viewer::TextPage, length: u64) -> (Worker, Mailbox) {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    worker.wrap = false;
    worker.raster.enabled = false;
    worker.length = length;
    worker.geometry = ViewerGeometry {
        width: 900,
        height: 1,
        scale: 1.0,
    };
    worker.positions.reset().unwrap();
    assert!(worker.set_page(Page::text(page.clone(), &worker.appearance), &output));
    let rows = worker.shaped.as_ref().unwrap().scroll_anchors();
    assert!(
        rows.len() >= 9,
        "eight authored LF rows need real row boundaries"
    );
    // Keep the viewport near six actual rows on the platform's chosen font.
    // Rounding down leaves the two edge rows in the ordinary Source budget.
    worker.geometry.height = (rows[6].1 - rows[0].1).floor().max(1.0) as u32;
    assert!(worker.set_page(Page::text(page, &worker.appearance), &output));
    assert!(worker.scroll_map.is_some());
    assert_eq!(worker.geometry.width, 900);
    assert!(!worker.raster.enabled);
    (worker, output)
}

#[test]
fn scrollbar_source_uneven_rows_correct_once_and_complete_the_actual_camera() {
    let source = mapped_prefix_source(false);
    let page = mapped_prefix_page(&source, 4000);
    assert_eq!(page.source.length, 310);
    assert_eq!(page.text.bytes().filter(|&byte| byte == b'\n').count(), 8);
    let (mut worker, output) = mapped_prefix_worker(page, source.len() as u64);
    let first = rendered(&mut worker, &output);
    // This source point lies after the short leading rows, yet before the
    // long A row ends. It cannot be displayed by the initial bounded layout.
    let target = (4000.0 + 35.5) / (20_000.0 - 306.0);
    let maximum = crate::viewport::available_height(
        worker.page.as_ref().unwrap(),
        worker.shaped.as_ref().unwrap(),
        worker.geometry.height as f32,
    ) - worker.geometry.height as f32;
    assert!(
        worker
            .scroll_map
            .as_ref()
            .unwrap()
            .inverse_near(
                target,
                f64::from(worker.geometry.height),
                0.0,
                f64::from(maximum),
                0.0
            )
            .is_none(),
        "the authored target must need a replacement fragment"
    );
    assert!(worker.positions.accept(first.generation, 4, false, target));
    read_target(&mut worker, 91, 4, target);
    let super::super::position::LoadedPosition::Reload {
        sequence,
        anchor,
        backwards,
    } = worker.refine_position()
    else {
        panic!("valid uneven Source rows must get one useful correction");
    };
    assert_eq!(sequence, 4);
    assert!(!backwards);
    let corrected = mapped_prefix_page(&source, anchor);
    assert!(corrected.text.starts_with('A'));
    assert!(corrected.partial_line);
    worker.positions.requested(92, sequence);
    worker.viewport_request = Some(92);
    worker.event(92, EventKind::TextPage(corrected), &output);
    assert!(worker.positions.has_pending());
    assert_eq!(worker.positions.acknowledged[0], 0);
    worker.event(92, EventKind::Completed, &output);
    let frame = rendered(&mut worker, &output);
    assert_eq!(frame.generation, first.generation);
    assert_eq!(frame.position_sequence[0], 4);
    assert!((frame.position - target).abs() < 0.0001);
    let actual = worker
        .scroll_map
        .as_ref()
        .unwrap()
        .metrics(
            f64::from(worker.origin[1]),
            f64::from(worker.geometry.height),
        )
        .unwrap();
    assert!((actual.0 - target).abs() < 0.0001);
    assert!(worker.origin[1] > 0.0);
    assert!(worker.viewport_request.is_none());
    assert!(!worker.positions.has_pending());
    assert_eq!(
        frame.pixels.len(),
        worker.geometry.width as usize * worker.geometry.height as usize * 4
    );
    assert!(
        frame.pixels != first.pixels,
        "the acknowledged authored camera must differ from the initial pixels"
    );
    assert!(
        frame
            .pixels
            .chunks_exact(4)
            .any(|pixel| pixel != &frame.pixels[..4]),
        "the acknowledged camera must render the authored content"
    );
}

#[test]
fn scrollbar_mapped_leading_plateau_keeps_a_zero_byte_prefix() {
    let source = mapped_prefix_source(false);
    let mut page = mapped_prefix_page(&source, 4000);
    // Public TextPage anchors may map generated/blank visual content to one
    // source boundary. This is a valid plateau, rather than missing mapping.
    page.source.length -= 6;
    for anchor in &mut page.anchors {
        anchor.source_offset = page.source.offset + u64::from(anchor.text_offset.saturating_sub(6));
    }
    midnight_plugin_host::api::codec::Model::validate(&page).unwrap();
    let (mut worker, output) = mapped_prefix_worker(page, source.len() as u64);
    let target = 0.7;
    let unprefixed =
        navigation::scrollbar_viewport(target, worker.length, worker.position_metrics().unwrap().1)
            .0;
    read_target(&mut worker, 91, 1, target);
    let super::super::position::LoadedPosition::Reload {
        anchor, backwards, ..
    } = worker.refine_position()
    else {
        panic!("the distant target needs one bounded request");
    };
    assert!(!backwards);
    assert_eq!(
        anchor, unprefixed,
        "a valid plateau does not invent source bytes"
    );
    worker.event(91, EventKind::Completed, &output);
    assert_eq!(worker.positions.acknowledged[0], 0);
    assert!(worker.status.contains("not completed"));
}

#[test]
fn scrollbar_dense_leading_rows_keep_the_quarter_page_prefix_cap() {
    let source = mapped_prefix_source(true);
    let page = mapped_prefix_page(&source, 4000);
    let byte_cap = page.source.length.div_ceil(4);
    let (mut worker, output) = mapped_prefix_worker(page, source.len() as u64);
    let target = 0.7;
    let unprefixed =
        navigation::scrollbar_viewport(target, worker.length, worker.position_metrics().unwrap().1)
            .0;
    read_target(&mut worker, 91, 1, target);
    let super::super::position::LoadedPosition::Reload {
        anchor, backwards, ..
    } = worker.refine_position()
    else {
        panic!("the distant target needs one bounded request");
    };
    assert!(!backwards);
    assert_eq!(unprefixed - anchor, byte_cap);
    assert_eq!(worker.positions.acknowledged[0], 0);
    worker.event(91, EventKind::Completed, &output);
    assert_eq!(worker.positions.acknowledged[0], 0);
    assert!(worker.viewport_request.is_none());
    assert!(worker.status.contains("not completed"));
}

#[test]
fn scrollbar_mapped_huge_offsets_stay_bounded_and_refuse_uncovered_completion() {
    let source = mapped_prefix_source(false);
    let shift = 1_u64 << 60;
    let mut page = mapped_prefix_page(&source, 4000);
    let byte_cap = page.source.length.div_ceil(4);
    page.source.offset += shift;
    for anchor in &mut page.anchors {
        anchor.source_offset += shift;
    }
    midnight_plugin_host::api::codec::Model::validate(&page).unwrap();
    let (mut worker, output) = mapped_prefix_worker(page, shift + source.len() as u64);
    let generation = worker.positions.generation;
    let target = 0.5;
    let unprefixed =
        navigation::scrollbar_viewport(target, worker.length, worker.position_metrics().unwrap().1)
            .0;
    read_target(&mut worker, 91, 4, target);
    let super::super::position::LoadedPosition::Reload {
        anchor, backwards, ..
    } = worker.refine_position()
    else {
        panic!("huge source offsets still allow one bounded correction");
    };
    assert!(!backwards);
    assert!(anchor > u64::from(u32::MAX));
    assert!(anchor <= unprefixed);
    assert!(unprefixed - anchor <= byte_cap);
    assert_eq!(worker.positions.generation, generation);
    assert_eq!(worker.positions.acknowledged[0], 0);
    assert_eq!(
        worker.refine_position(),
        super::super::position::LoadedPosition::Unavailable
    );
    worker.event(91, EventKind::Completed, &output);
    let frame = rendered(&mut worker, &output);
    assert_eq!(frame.position_sequence[0], 0);
    assert_ne!(frame.generation, generation);
    assert!(worker.viewport_request.is_none());
    assert!(worker.status.contains("not completed"));
}

#[test]
fn scrollbar_prefix_preserves_absolute_endpoints_with_and_without_a_map() {
    for mapped in [false, true] {
        for position in [0.0, 1.0] {
            let source = mapped_prefix_source(false);
            let page = mapped_prefix_page(&source, 4000);
            let (mut worker, _) = mapped_prefix_worker(page, source.len() as u64);
            if !mapped {
                worker.scroll_map = None;
            }
            read_target(&mut worker, 91, 1, position);
            let super::super::position::LoadedPosition::Reload {
                anchor, backwards, ..
            } = worker.refine_position()
            else {
                panic!("a partial middle fragment cannot answer a real file endpoint");
            };
            if position == 0.0 {
                assert_eq!(anchor, 0);
                assert!(!backwards);
            } else {
                assert_eq!(anchor, worker.length);
                assert!(backwards);
            }
            assert_eq!(worker.positions.acknowledged[0], 0);
        }
    }
}

#[test]
fn scrollbar_no_map_fallback_keeps_the_loaded_visual_camera_without_a_read() {
    let source = mapped_prefix_source(false);
    let page = mapped_prefix_page(&source, 4000);
    let (mut worker, output) = mapped_prefix_worker(page, source.len() as u64);
    worker.scroll_map = None;
    let page = worker.page.as_ref().unwrap();
    let shaped = worker.shaped.as_ref().unwrap();
    let maximum = crate::viewport::available_height(page, shaped, worker.geometry.height as f32)
        - worker.geometry.height as f32;
    let wanted_y = maximum * 0.4;
    let target = navigation::scroll_metrics(
        page.start,
        page.end,
        worker.length,
        shaped.height(),
        worker.geometry.height as f32,
        wanted_y,
    )
    .0;
    let generation = worker.positions.generation;
    worker.input(
        ViewerInput::Position {
            generation,
            sequence: 4,
            horizontal: false,
            position: target,
        },
        &output,
    );
    let frame = rendered(&mut worker, &output);
    assert_eq!(frame.generation, generation);
    assert_eq!(frame.position_sequence[0], 4);
    assert!((frame.position - target).abs() < 0.0001);
    assert!((worker.origin[1] - wanted_y).abs() < 0.001);
    assert!(worker.scroll_map.is_none());
    assert!(worker.viewport_request.is_none());
}

// The raster test module's crop helper is private to that sibling module.
// Compose the actual visible body and pinned gutter, preserving pixel phase.
fn corrected_source_visible_pixels(frame: &ViewerFrame) -> Vec<u8> {
    let info = frame.raster.as_ref().expect("actual retained Source frame");
    assert!(info.settled, "the corrected Source camera must be settled");
    let gutter = info.gutter.as_ref().expect("pinned Source line numbers");
    assert!(gutter.width > 0 && gutter.width < info.viewport[0]);
    let dx = info.origin[0] - info.raster_origin[0];
    let dy = info.origin[1] - info.raster_origin[1];
    assert!(
        dx >= 0.0 && dy >= 0.0 && (dx - dx.round()).abs() < 1e-5 && (dy - dy.round()).abs() < 1e-5,
        "settled Source pixels must permit an exact crop: {dx},{dy}"
    );
    let [width, height] = info.viewport;
    let mut visible = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        let source_y = y + dy.round() as usize;
        let gutter_at = source_y * gutter.width as usize * 4;
        visible.extend_from_slice(&gutter.pixels[gutter_at..gutter_at + gutter.width as usize * 4]);
        let body_at = (source_y * frame.width as usize + dx.round() as usize) * 4;
        visible.extend_from_slice(
            &frame.pixels[body_at..body_at + (width - gutter.width) as usize * 4],
        );
    }
    visible
}

#[test]
fn scrollbar_source_correction_retains_actual_pixels_and_exact_numbered_gutter() {
    let source = mapped_prefix_source(false);
    let page = mapped_prefix_page(&source, 4000);
    let (mut worker, output) = mapped_prefix_worker(page, source.len() as u64);
    worker.input(ViewerInput::RetainedRaster(true), &output);

    // The owned prefix contains exactly 2,000 LFs. Use the real Locate event
    // path so the pixel comparison includes numbered, rather than blank, gutters.
    worker.text.locate_request = Some((90, 4000));
    worker.event(
        90,
        EventKind::TextPosition(viewer::TextPosition {
            revision: 1,
            source_offset: 4000,
            line: 2001,
            column: 1,
        }),
        &output,
    );
    worker.event(90, EventKind::Completed, &output);
    let first = rendered(&mut worker, &output);
    let first_visible = corrected_source_visible_pixels(&first);
    let generation = first.generation;
    let target = (4000.0 + 35.5) / (20_000.0 - 306.0);

    // This unit fixture models admitted request arrival, as do the existing
    // correction tests. It has no live Session or uncovered Position dispatcher.
    // No acknowledgement is set here: Worker::event must produce it on completion.
    assert!(worker.positions.accept(generation, 4, false, target));
    read_target(&mut worker, 91, 4, target);
    let super::super::position::LoadedPosition::Reload {
        sequence,
        anchor,
        backwards,
    } = worker.refine_position()
    else {
        panic!("the nonuniform Source target must require one corrective page");
    };
    assert_eq!(sequence, 4);
    assert!(!backwards);
    let corrected = mapped_prefix_page(&source, anchor);
    assert!(corrected.text.starts_with('A') && corrected.partial_line);
    worker.positions.requested(92, sequence);
    worker.viewport_request = Some(92);
    worker.event(92, EventKind::TextPage(corrected), &output);
    assert!(worker.positions.has_pending());
    assert_eq!(worker.positions.acknowledged[0], 0);
    worker.paint(&output).unwrap();
    assert!(
        !output
            .take()
            .iter()
            .any(|message| matches!(message, ViewerOutput::Frame(_))),
        "an unterminated correction must not publish acknowledged pixels"
    );
    worker.event(92, EventKind::Completed, &output);
    let frame = rendered(&mut worker, &output);
    assert_eq!(frame.generation, generation);
    assert_eq!(frame.position_sequence[0], 4);
    assert!((frame.position - target).abs() < 0.0001);
    assert!(worker.origin[1] > 0.0);
    assert!(worker.viewport_request.is_none() && !worker.positions.has_pending());

    let coordinate = worker
        .text
        .coordinate
        .expect("transferred Source coordinate");
    let prefix = &source[..anchor as usize];
    assert_eq!(coordinate.source_offset, anchor);
    assert_eq!(
        coordinate.line,
        prefix.bytes().filter(|&b| b == b'\n').count() as u64 + 1
    );
    assert_eq!(
        coordinate.column,
        Some(prefix.rsplit('\n').next().unwrap().len() as u64 + 1)
    );
    let visible = corrected_source_visible_pixels(&frame);
    let info = frame.raster.as_ref().unwrap();
    let mapped_position = info
        .scroll_map
        .as_ref()
        .expect("retained Source map")
        .metrics(info.origin[1], f64::from(info.viewport[1]))
        .unwrap()
        .0;
    assert!((mapped_position - target).abs() < 0.0001);
    assert!(
        visible != first_visible,
        "the correction must replace the initial pixels"
    );
    let gutter_bytes = info.gutter.as_ref().unwrap().width as usize * 4;
    assert!(
        visible
            .chunks_exact(worker.geometry.width as usize * 4)
            .any(|row| {
                row[..gutter_bytes]
                    .chunks_exact(4)
                    .any(|pixel| pixel != &row[..4])
            }),
        "the actual visible Source gutter must contain line-number glyphs"
    );

    let origin = worker.origin;
    worker.input(ViewerInput::RetainedRaster(false), &output);
    assert_eq!(worker.origin, origin);
    let reference = rendered(&mut worker, &output);
    assert!(reference.raster.is_none());
    assert_eq!(reference.generation, generation);
    assert_eq!(reference.position_sequence[0], 4);
    assert_eq!(visible.len(), reference.pixels.len());
    let first_mismatch = visible
        .iter()
        .zip(reference.pixels.iter())
        .position(|(actual, expected)| actual != expected);
    assert_eq!(
        first_mismatch, None,
        "corrected retained body and numbered gutter must match the CPU viewport"
    );
}
