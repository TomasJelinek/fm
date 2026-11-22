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

pub(super) fn rendered(worker: &mut Worker, output: &Mailbox) -> ViewerFrame {
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
pub(super) fn mapped_prefix_page(source: &str, anchor: u64) -> viewer::TextPage {
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

pub(super) fn mapped_prefix_worker(page: viewer::TextPage, length: u64) -> (Worker, Mailbox) {
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

/// Model transport admission while letting the real Source resolver request
/// complete leading context before any pixels or position can be acknowledged.
fn mapped_prefix_recovery_page(
    worker: &mut Worker,
    output: &Mailbox,
    source: &str,
    target: f64,
) -> viewer::TextPage {
    use super::super::position::SourcePosition;

    let (anchor, backwards) = worker.position_page_request(target);
    assert!(!backwards);
    let suffix = mapped_prefix_page(source, anchor);
    assert!(suffix.text.starts_with('A') && suffix.partial_line);
    let end = suffix.source.end().unwrap();
    recovery_integration_admit(worker, 91, 4, target, anchor, 8);
    worker.event(91, EventKind::TextPage(suffix), output);
    assert!(worker.positions.has_pending());
    assert_eq!(worker.positions.acknowledged[0], 0);
    let (request, candidate) = worker.position_page.take().unwrap();
    assert_eq!(request, 91);
    let SourcePosition::Reload { sequence, viewport } =
        worker.resolve_source_position(candidate, output)
    else {
        panic!("the partial A row must recover its complete prefix before acknowledgement");
    };
    assert_eq!(sequence, 4);
    assert!(viewport.backwards);
    assert_eq!(viewport.source_anchor, end);
    assert_eq!(viewport.max_lines, 16);
    assert_eq!(viewport.max_bytes, 12_288);
    assert_eq!(worker.positions.acknowledged[0], 0);
    // The provider's backwards reply retains the suffix endpoint and includes
    // sixteen authored LF rows, including the complete first A row and prefix.
    let end = end as usize;
    let length: usize = source[..end]
        .split_inclusive('\n')
        .rev()
        .take(viewport.max_lines as usize)
        .map(str::len)
        .sum();
    let page = recovery_v2_ascii_page(source, end - length, end);
    assert!(page.source.offset < anchor && !page.partial_line);
    assert_eq!(page.text.bytes().filter(|&b| b == b'\n').count(), 16);
    page
}

#[test]
fn scrollbar_source_uneven_rows_recover_the_prefix_and_complete_the_actual_camera() {
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
    let corrected = mapped_prefix_recovery_page(&mut worker, &output, &source, target);
    worker.positions.requested(92, 4);
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
            .as_chunks::<4>()
            .0
            .iter()
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
    let (frame, unavailable) = recovery_v2_observed_frame(&mut worker, &output);
    assert_eq!(frame.position_sequence[0], 0);
    assert_eq!(frame.generation, generation);
    assert_eq!(unavailable, vec![(generation, 4)]);
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

/// The window rests on whole pixels, where prepared strips are exact, and asks the worker
/// for that camera: settle `frame`'s camera there as the window would.
fn settle_whole(worker: &mut Worker, output: &Mailbox, frame: ViewerFrame) -> ViewerFrame {
    let info = frame.raster.as_ref().expect("retained frame");
    let whole = info.origin.map(f64::round);
    if whole == info.origin {
        return frame;
    }
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::PanTo {
                sequence: info.pan_sequence + 1,
                origin: whole,
                settled: true,
            },
        },
        output,
    );
    rendered(worker, output)
}

/// The actual visible body and pinned gutter of a settled retained Source frame, composed
/// from its prepared strips (`raster::stacked`).
fn corrected_source_visible_pixels(frame: &ViewerFrame) -> Vec<u8> {
    let info = frame.raster.as_ref().expect("actual retained Source frame");
    assert!(info.settled, "the corrected Source camera must be settled");
    let (body_width, body, gutter) = super::raster::stacked(frame);
    let (gutter_width, gutter) = gutter.expect("pinned Source line numbers");
    assert!(gutter_width > 0 && gutter_width < info.viewport[0]);
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
        let gutter_at = source_y * gutter_width as usize * 4;
        visible.extend_from_slice(&gutter[gutter_at..gutter_at + gutter_width as usize * 4]);
        let body_at = (source_y * body_width as usize + dx.round() as usize) * 4;
        visible.extend_from_slice(&body[body_at..body_at + (width - gutter_width) as usize * 4]);
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

    // The partial first response needs its complete leading prefix. Keep the
    // real SourceRecovery history; Worker::event owns completion acknowledgement.
    let corrected = mapped_prefix_recovery_page(&mut worker, &output, &source, target);
    worker.positions.requested(92, 4);
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
    // Prefix expansion still contains the exact Locate result established
    // before the seek; do not manufacture a new column for its earlier rows.
    let prefix = &source[..4000];
    assert_eq!(coordinate.source_offset, 4000);
    assert_eq!(
        coordinate.line,
        prefix.bytes().filter(|&b| b == b'\n').count() as u64 + 1
    );
    assert_eq!(
        coordinate.column,
        Some(prefix.rsplit('\n').next().unwrap().len() as u64 + 1)
    );
    // The exact corrected camera maps to the target; the window then rests on the nearest
    // whole pixel, where the strips' pixels are exact.
    let corrected = frame.raster.clone().unwrap();
    let frame = settle_whole(&mut worker, &output, frame);
    let visible = corrected_source_visible_pixels(&frame);
    let info = frame.raster.as_ref().unwrap();
    let mapped_position = corrected
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
    let gutter_bytes = info.bands.as_ref().unwrap().gutter_width as usize * 4;
    assert!(
        visible
            .chunks_exact(worker.geometry.width as usize * 4)
            .any(|row| {
                row[..gutter_bytes]
                    .as_chunks::<4>()
                    .0
                    .iter()
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

#[test]
#[ignore = "requires MIDNIGHT_VIEWER_TEST_INSTALL with the trusted installed Source plugin"]
fn installed_source_correction_advances_the_line_window_before_acknowledging() {
    const LENGTH: usize = 75_881;
    const FIRST_ANCHOR: u64 = 7641;
    const FIRST_END: u64 = 8134;
    const TARGET: f64 = 0.1025;
    const SEQUENCE: u64 = 4;

    #[derive(Clone, Copy)]
    struct Response {
        request: u64,
        start: u64,
        end: u64,
        lines: usize,
    }

    struct Finished {
        requests: Vec<u64>,
        responses: Vec<Response>,
        completed: Vec<u64>,
        generation: u64,
        frame: ViewerFrame,
        unavailable: Vec<(u64, u64)>,
        actual_position: f64,
        minimum_position: f64,
        maximum_position: f64,
        pending: bool,
        viewport: Option<u64>,
    }

    fn response(request: u64, page: &viewer::TextPage, source: &str) -> Response {
        let end = page
            .source
            .end()
            .expect("owned Source range must not overflow");
        let start_index = usize::try_from(page.source.offset).unwrap();
        let end_index = usize::try_from(end).unwrap();
        assert!(
            start_index > 0,
            "the owned response must start inside the fixture"
        );
        assert!(
            end_index <= source.len(),
            "Source response exceeds the owned fixture"
        );
        assert!(
            page.text.as_bytes() == &source.as_bytes()[start_index..end_index],
            "actual Source response must equal its owned source slice"
        );
        let lines = page.text.bytes().filter(|&byte| byte == b'\n').count();
        assert!(
            (1..=512).contains(&lines),
            "actual Source recovery must retain the bounded line budget"
        );
        assert!(
            page.text.len() < 8192,
            "the fixture must remain below Source's byte cap"
        );
        assert!(page.has_before && page.has_after);
        assert_eq!(
            page.partial_line,
            source.as_bytes()[start_index - 1] != b'\n',
            "the partial-line flag must describe the actual owned crop"
        );
        Response {
            request,
            start: page.source.offset,
            end,
            lines,
        }
    }

    fn check_budget(worker: &mut Worker) {
        let viewport = worker.page_viewport(FIRST_ANCHOR, false);
        assert_eq!(viewport.source_anchor, FIRST_ANCHOR);
        assert!(!viewport.backwards);
        assert_eq!(viewport.max_bytes, 12_288);
        assert_eq!(
            viewport.max_lines, 10,
            "real font/geometry must reproduce the setup"
        );
    }

    fn next_events(worker: &Worker) -> Vec<midnight_plugin_host::HostEvent> {
        worker
            .session
            .as_ref()
            .expect("actual Source session")
            .drain(32)
    }

    fn wait_open(worker: &Worker, deadline: std::time::Instant) {
        let mut opened = false;
        let mut completed = false;
        while !(opened && completed) {
            assert!(
                std::time::Instant::now() < deadline,
                "owned Source open did not finish"
            );
            let events = next_events(worker);
            if events.is_empty() {
                std::thread::sleep(Duration::from_millis(1));
            }
            for event in events {
                assert_eq!(event.generation, 1, "Source startup protocol generation");
                assert_eq!(
                    event.request_id, 1,
                    "only the actual Open request is pending"
                );
                match event.kind {
                    EventKind::Opened(open) => {
                        assert_eq!(open.mode, viewer::MODE_SOURCE);
                        assert_eq!(open.length, Some(LENGTH as u64));
                        opened = true;
                    }
                    EventKind::Completed => completed = true,
                    EventKind::Failed(failure) => {
                        panic!("owned Source open failed with code {}", failure.code);
                    }
                    _ => panic!("unexpected owned Source startup event"),
                }
            }
        }
        // Opened is deliberately not fed to Worker: that would admit page zero.
    }

    fn with_source_worker<T>(
        runtime: &Arc<Runtime>,
        path: &std::path::Path,
        run: impl FnOnce(&mut Worker, &Mailbox, std::time::Instant) -> T,
    ) -> T {
        let captured = midnight_plugin_host::CapturedFile::capture(path)
            .expect("capture only the owned ASCII Source fixture");
        assert_eq!(captured.length(), LENGTH as u64);
        let (mut worker, output) = worker();
        worker.runtime = runtime.clone();
        worker.files = vec![path.to_path_buf()].into();
        worker.captured = Some((0, Ok(captured.clone())));
        worker.mode = viewer::MODE_SOURCE;
        worker.modes = vec![viewer::MODE_SOURCE];
        worker.generation = 1;
        worker.positions.reset().unwrap();
        worker.length = LENGTH as u64;
        worker.geometry = ViewerGeometry {
            width: 900,
            height: 120,
            scale: 1.0,
        };
        worker.wrap = false;
        worker.raster.enabled = false;
        worker.session = Some(
            runtime
                .open("midnight.viewer.source", captured, 1, viewer::MODE_SOURCE)
                .expect("open the trusted installed Source plugin"),
        );
        // Each phase gets one shared 15-second component deadline. Cleanup has
        // its own ten-second bound; original Source410 waits are untouched.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            wait_open(&worker, deadline);
            check_budget(&mut worker);
            run(&mut worker, &output, deadline)
        }));
        let session = worker
            .session
            .take()
            .expect("retain Source session for cleanup");
        session.close();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut generation_matches = true;
        let mut closed_and_drained = false;
        while std::time::Instant::now() < deadline {
            let events = session.drain(32);
            generation_matches &= events.iter().all(|event| event.generation == 1);
            if session.is_closed() && events.is_empty() {
                closed_and_drained = true;
                break;
            }
            if events.is_empty() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        drop(session);
        drop(worker);
        assert!(
            closed_and_drained,
            "owned Source session did not close and drain"
        );
        assert!(generation_matches, "Source cleanup protocol generation");
        match result {
            Ok(value) => value,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    fn bootstrap(
        worker: &mut Worker,
        output: &Mailbox,
        source: &str,
        deadline: std::time::Instant,
    ) {
        worker.request_page_for(FIRST_ANCHOR, false, None, output);
        let request = worker
            .viewport_request
            .expect("admit the actual bootstrap viewport");
        let mut received = false;
        let mut completed = false;
        while !completed {
            assert!(
                std::time::Instant::now() < deadline,
                "Source bootstrap did not finish"
            );
            let events = next_events(worker);
            if events.is_empty() {
                std::thread::sleep(Duration::from_millis(1));
            }
            for event in events {
                assert_eq!(event.generation, worker.generation);
                assert_eq!(event.request_id, request, "bootstrap request identity");
                if let EventKind::TextPage(page) = &event.kind {
                    let observed = response(request, page, source);
                    assert_eq!(observed.start, FIRST_ANCHOR);
                    assert_eq!(observed.end, FIRST_END);
                    assert_eq!(observed.lines, 10);
                    assert_eq!(page.text.len(), 493);
                    assert!(
                        page.partial_line,
                        "the owned prefix ends inside a logical row"
                    );
                    received = true;
                }
                if matches!(event.kind, EventKind::Completed) {
                    assert!(received, "the real Source page must precede Completed");
                    completed = true;
                }
                assert!(
                    !matches!(event.kind, EventKind::Failed(_) | EventKind::Cancelled),
                    "bootstrap must be actual successful Source delivery"
                );
                worker.event(event.request_id, event.kind, output);
            }
        }
        assert!(worker.viewport_request.is_none());
        assert!(!worker.positions.has_pending());
    }

    fn finish_position(
        worker: &mut Worker,
        output: &Mailbox,
        source: &str,
        deadline: std::time::Instant,
        controlled_first: bool,
    ) -> Finished {
        let generation = worker.positions.generation;
        let first = worker
            .viewport_request
            .expect("actual position request admitted");
        let mut requests = vec![first];
        let mut responses = Vec::new();
        let mut completed = Vec::new();
        while worker.positions.has_pending() {
            assert!(
                std::time::Instant::now() < deadline,
                "owned Source position did not finish"
            );
            let events = next_events(worker);
            if events.is_empty() {
                std::thread::sleep(Duration::from_millis(1));
            }
            for event in events {
                assert_eq!(
                    event.generation, worker.generation,
                    "position protocol generation"
                );
                assert!(
                    requests.contains(&event.request_id),
                    "only admitted viewport callbacks"
                );
                let request = event.request_id;
                let is_page = matches!(event.kind, EventKind::TextPage(_));
                let is_completed = matches!(event.kind, EventKind::Completed);
                assert!(
                    !matches!(event.kind, EventKind::Failed(_) | EventKind::Cancelled),
                    "actual Source viewport must reach successful producer completion"
                );
                if let EventKind::TextPage(page) = &event.kind {
                    let observed = response(request, page, source);
                    if controlled_first && request == first {
                        assert_eq!(observed.start, FIRST_ANCHOR);
                        assert_eq!(observed.end, FIRST_END);
                        assert_eq!(observed.lines, 10);
                        assert_eq!(page.text.len(), 493);
                        assert!(page.partial_line);
                    }
                    assert!(
                        !responses
                            .iter()
                            .any(|previous: &Response| previous.request == request),
                        "one actual Source page per viewport"
                    );
                    responses.push(observed);
                }
                if is_completed {
                    assert!(
                        responses.iter().any(|page| page.request == request),
                        "actual TextPage must precede its terminal completion"
                    );
                    assert_ne!(worker.positions.acknowledged[0], SEQUENCE);
                    completed.push(request);
                }
                worker.event(request, event.kind, output);
                if is_page {
                    assert_ne!(worker.positions.acknowledged[0], SEQUENCE);
                    worker.paint(output).unwrap();
                    assert!(
                        !output
                            .take()
                            .iter()
                            .any(|message| matches!(message, ViewerOutput::Frame(_))),
                        "unterminated Source page must not publish a success frame"
                    );
                }
                if is_completed
                    && let Some(next) = worker.viewport_request
                    && next != request
                    && !requests.contains(&next)
                {
                    requests.push(next);
                    assert!(requests.len() <= 4, "bounded Source recovery requests only");
                }
            }
        }
        let (frame, unavailable) = recovery_v2_observed_frame(worker, output);
        let shaped = worker.shaped.as_ref().expect("actual Source layout");
        let page = worker.page.as_ref().expect("actual Source page");
        let maximum =
            (crate::viewport::available_height(page, shaped, worker.geometry.height as f32)
                - worker.geometry.height as f32)
                .max(0.0);
        let map = worker.scroll_map.as_ref().expect("actual Source row map");
        let actual_position = map
            .metrics(
                f64::from(worker.origin[1]),
                f64::from(worker.geometry.height),
            )
            .expect("actual displayed Source metrics")
            .0;
        let minimum_position = map
            .metrics(0.0, f64::from(worker.geometry.height))
            .expect("actual Source low endpoint")
            .0;
        let maximum_position = map
            .metrics(f64::from(maximum), f64::from(worker.geometry.height))
            .expect("actual Source high endpoint")
            .0;
        Finished {
            requests,
            responses,
            completed,
            generation,
            frame,
            unavailable,
            actual_position,
            minimum_position,
            maximum_position,
            pending: worker.positions.has_pending(),
            viewport: worker.viewport_request,
        }
    }

    let installation = std::path::PathBuf::from(
        std::env::var_os("MIDNIGHT_VIEWER_TEST_INSTALL")
            .expect("trusted Source installation required"),
    );
    let runtime =
        Arc::new(Runtime::discover(&installation).expect("discover trusted installed viewers"));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("owned_nonuniform_source.cpp");
    let mut source = "p\n".repeat(3820);
    source.push('p');
    assert_eq!(source.len(), FIRST_ANCHOR as usize);
    for (index, length) in [64, 5, 1, 50, 50, 50, 100, 100, 40, 33]
        .into_iter()
        .enumerate()
    {
        source.extend(std::iter::repeat_n(
            char::from(b'A' + index as u8),
            length - 1,
        ));
        source.push('\n');
    }
    assert_eq!(source.len(), FIRST_END as usize);
    source.push_str(&"z\n".repeat((LENGTH - source.len()) / 2));
    if source.len() < LENGTH {
        source.push('z');
    }
    assert_eq!(source.len(), LENGTH);
    assert!(source.is_ascii());
    std::fs::write(&path, source.as_bytes()).unwrap();

    with_source_worker(&runtime, &path, |worker, output, deadline| {
        bootstrap(worker, output, &source, deadline);
        let first = rendered(worker, output);
        let baseline_high = {
            let page = worker.page.as_ref().expect("actual bootstrap Source page");
            let shaped = worker
                .shaped
                .as_ref()
                .expect("actual bootstrap Source layout");
            let maximum =
                (crate::viewport::available_height(page, shaped, worker.geometry.height as f32)
                    - worker.geometry.height as f32)
                    .max(0.0);
            worker
                .scroll_map
                .as_ref()
                .expect("actual bootstrap Source map")
                .metrics(f64::from(maximum), f64::from(worker.geometry.height))
                .expect("actual bootstrap Source high endpoint")
                .0
        };
        // Actual Position dispatch invokes the ordinary first calculation.
        // This explicit legacy negative exhausts the historical one-correction
        // path; the admitted producer request keeps its actual ten-line budget.
        worker.input(
            ViewerInput::Position {
                generation: first.generation,
                sequence: SEQUENCE,
                horizontal: false,
                position: TARGET,
            },
            output,
        );
        let target = worker
            .positions
            .target
            .as_mut()
            .expect("actual uncovered Position intent");
        assert_eq!(target.sequence, SEQUENCE);
        assert_eq!(target.position, TARGET);
        assert!(!target.corrected);
        target.corrected = true;
        worker.positions.source = None;
        let negative = finish_position(worker, output, &source, deadline, false);
        assert_eq!(negative.requests.len(), 1);
        assert_eq!(negative.completed.len(), 1);
        assert_eq!(negative.responses.len(), 1);
        let response = negative.responses[0];
        assert!(response.start > FIRST_ANCHOR && response.start < FIRST_ANCHOR + 64);
        assert_eq!(
            response.end, FIRST_END,
            "the control retains the same tenth LF"
        );
        assert_eq!(negative.maximum_position, baseline_high);
        assert!(negative.maximum_position < TARGET);
        // The window's cameras end short of the target, so the exhausted correction
        // settles at its nearest edge (`settle_at_nearest_edge`) and acknowledges
        // there instead of reporting the position unavailable.
        assert_eq!(negative.frame.generation, negative.generation);
        assert_eq!(negative.frame.position_sequence[0], SEQUENCE);
        assert!(negative.unavailable.is_empty());
        assert_eq!(negative.actual_position, negative.maximum_position);
        assert_eq!(negative.frame.position, negative.actual_position);
        assert!(!negative.pending && negative.viewport.is_none());
    });

    let positive = with_source_worker(&runtime, &path, |worker, output, deadline| {
        // This models the trace's admitted first target read, not initial UI
        // dispatch. Every page, layout, correction admission and ACK is real.
        let generation = worker.positions.generation;
        assert!(worker.positions.accept(generation, SEQUENCE, false, TARGET));
        worker.positions.target = Some(crate::position_ack::Target {
            sequence: SEQUENCE,
            position: TARGET,
            corrected: false,
        });
        worker.request_page_for(FIRST_ANCHOR, false, Some(SEQUENCE), output);
        finish_position(worker, output, &source, deadline, true)
    });
    // Both actual sessions are now closed and drained, even when the following
    // old-production behavioral oracle fails. No successful anchor was chosen here.
    assert_eq!(
        positive.requests.len(),
        2,
        "the same-tail fixture needs its initial request and one covering recovery"
    );
    assert_eq!(
        positive.completed.len(),
        2,
        "both actual producer Completeds were consumed"
    );
    assert_eq!(positive.responses.len(), 2);
    assert_eq!(positive.completed, positive.requests);
    assert_eq!(positive.responses[0].end, FIRST_END);
    assert_eq!(
        positive.frame.generation, positive.generation,
        "a real completed correction must retain the admitted position generation"
    );
    assert_eq!(positive.frame.position_sequence[0], SEQUENCE);
    assert!(positive.unavailable.is_empty());
    assert!(positive.responses[1].end > positive.responses[0].end);
    assert!(positive.minimum_position <= TARGET && TARGET <= positive.maximum_position);
    assert_eq!(positive.frame.position, positive.actual_position);
    assert_eq!(positive.actual_position as f32, TARGET as f32);
    assert!(!positive.pending && positive.viewport.is_none());
    let final_page = positive.responses[1];
    assert!(
        positive.frame.accessible_text.as_bytes()
            == &source.as_bytes()[final_page.start as usize..final_page.end as usize],
        "painted Source content must match the actual final producer response"
    );
    assert_eq!(positive.frame.width, 900);
    assert_eq!(positive.frame.height, 120);
    assert_eq!(positive.frame.pixels.len(), 900 * 120 * 4);
    assert!(positive.frame.raster.is_none());
    assert!(
        positive
            .frame
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel.as_slice() != &positive.frame.pixels[..4]),
        "the actual acknowledged Source camera must contain rendered glyphs"
    );
}

fn recovery_integration_page(source: &str, anchor: u64) -> viewer::TextPage {
    let start = anchor as usize;
    let text: String = source[start..].split_inclusive('\n').take(40).collect();
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
                source_offset: end as u64,
            }))
            .collect(),
        partial_line: start > 0 && source.as_bytes()[start - 1] != b'\n',
        text,
        spans: Vec::new(),
        has_before: start > 0,
        has_after: end < source.len(),
    };
    page.validate().unwrap();
    page
}

fn recovery_integration_worker() -> (Worker, Mailbox, String, ViewerFrame) {
    let source = mapped_prefix_source(false);
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    worker.generation = 1;
    worker.positions.reset().unwrap();
    worker.length = source.len() as u64;
    worker.geometry = ViewerGeometry {
        width: 350,
        height: 110,
        scale: 1.0,
    };
    worker.raster.enabled = false;
    assert!(worker.set_page(
        Page::text(recovery_integration_page(&source, 0), &worker.appearance),
        &output,
    ));
    let generation = worker.positions.generation;
    let position = position_at(&worker, 31.5);
    worker.input(
        ViewerInput::Position {
            generation,
            sequence: 3,
            horizontal: false,
            position,
        },
        &output,
    );
    let frame = rendered(&mut worker, &output);
    assert_eq!(frame.position_sequence[0], 3);
    assert!((worker.origin[1] - 31.5).abs() < 0.001);
    (worker, output, source, frame)
}

pub(super) fn recovery_integration_admit(
    worker: &mut Worker,
    request: u64,
    sequence: u64,
    position: f64,
    anchor: u64,
    max_lines: u32,
) {
    assert!(
        worker
            .positions
            .accept(worker.positions.generation, sequence, false, position)
    );
    read_target(worker, request, sequence, position);
    // read_target also models an already loaded legacy page. This fixture
    // represents admission only; the matching TextPage must supply the data.
    worker.positions.requested(request, sequence);
    worker.position_page = None;
    worker.positions.source = Some(crate::position_ack::SourceRecovery::new(
        messages::Viewport {
            source_anchor: anchor,
            backwards: false,
            max_bytes: 12_288,
            max_lines,
        },
    ));
}

fn recovery_integration_assert_display(
    worker: &mut Worker,
    output: &Mailbox,
    expected: &ViewerFrame,
    origin: [f32; 2],
    range: (u64, u64),
) {
    let page = worker
        .page
        .as_ref()
        .expect("the displayed Source page remains loaded");
    assert_eq!((page.start, page.end), range);
    assert!(page.text.as_ref() == expected.accessible_text.as_ref());
    assert_eq!(worker.origin, origin);
    assert_eq!(worker.positions.generation, expected.generation);
    assert_eq!(worker.positions.acknowledged, expected.position_sequence);
    worker.paint(output).unwrap();
    for message in output.take() {
        if let ViewerOutput::Frame(frame) = message {
            assert_eq!(frame.generation, expected.generation);
            assert_eq!(frame.position_sequence, expected.position_sequence);
            assert!(frame.accessible_text == expected.accessible_text);
            assert!(
                frame.pixels == expected.pixels,
                "staged or stale Source data changed pixels"
            );
        }
    }
}

fn recovery_integration_superseded(terminal: EventKind) -> (Worker, Mailbox, String, ViewerFrame) {
    let (mut worker, output, source, previous) = recovery_integration_worker();
    let generation = previous.generation;
    let origin = worker.origin;
    let old_range = {
        let page = worker.page.as_ref().unwrap();
        (page.start, page.end)
    };
    let newest_y = 92.25;
    let newest = position_at(&worker, newest_y);
    recovery_integration_admit(&mut worker, 91, 4, 0.5, 4000, 40);
    worker.event(
        91,
        EventKind::TextPage(recovery_integration_page(&source, 4000)),
        &output,
    );
    recovery_integration_assert_display(&mut worker, &output, &previous, origin, old_range);

    // The newer intent arrives after data but before the old terminal. It is
    // inside the still-displayed map, so no fake follow-up Session is needed.
    worker.input(
        ViewerInput::Position {
            generation,
            sequence: 5,
            horizontal: false,
            position: newest,
        },
        &output,
    );
    assert!(worker.positions.has_deferred());
    recovery_integration_assert_display(&mut worker, &output, &previous, origin, old_range);
    worker.event(91, terminal, &output);
    let current = rendered(&mut worker, &output);
    assert_eq!(
        worker.generation, 1,
        "the protocol session was never replaced"
    );
    assert_eq!(current.generation, generation);
    assert_eq!(current.position_sequence[0], 5);
    assert_eq!(current.position as f32, newest as f32);
    assert!((worker.origin[1] - newest_y).abs() < 0.001);
    assert!(current.accessible_text == previous.accessible_text);
    assert_eq!(
        worker.page.as_ref().map(|page| (page.start, page.end)),
        Some(old_range)
    );
    assert!(!worker.positions.has_pending());
    assert!(!worker.positions.has_deferred());
    assert!(worker.positions.source.is_none());
    assert!(worker.viewport_request.is_none());

    let current_origin = worker.origin;
    for late in [
        EventKind::Failed(messages::Failure {
            code: 1,
            message: "owned retired request failure".into(),
        }),
        EventKind::Cancelled,
        EventKind::TextPage(recovery_integration_page(&source, 4000)),
        EventKind::Completed,
    ] {
        worker.event(91, late, &output);
        recovery_integration_assert_display(
            &mut worker,
            &output,
            &current,
            current_origin,
            old_range,
        );
    }
    (worker, output, source, current)
}

#[test]
fn scrollbar_source_recovery_new_seek_between_data_and_completed_keeps_the_latest_camera() {
    let _ = recovery_integration_superseded(EventKind::Completed);
}

#[test]
fn scrollbar_source_recovery_superseded_failed_and_cancelled_requests_do_not_reset_generation() {
    for terminal in [
        EventKind::Failed(messages::Failure {
            code: 1,
            message: "owned superseded request failure".into(),
        }),
        EventKind::Cancelled,
    ] {
        let _ = recovery_integration_superseded(terminal);
    }
}

#[test]
fn scrollbar_source_recovery_late_old_callbacks_cannot_replace_the_active_candidate() {
    let (mut worker, output, source, previous) =
        recovery_integration_superseded(EventKind::Completed);
    let origin = worker.origin;
    let old_range = {
        let page = worker.page.as_ref().unwrap();
        (page.start, page.end)
    };
    let candidate = recovery_integration_page(&source, 1000);
    let (mut reference, reference_output) = super::worker();
    reference.mode = viewer::MODE_SOURCE;
    reference.length = source.len() as u64;
    reference.geometry = worker.geometry;
    reference.raster.enabled = worker.raster.enabled;
    assert!(reference.set_page(
        Page::text(candidate.clone(), &reference.appearance),
        &reference_output
    ));
    let wanted_y = 73.25;
    let target = position_at(&reference, wanted_y);
    reference.origin[1] = wanted_y;
    let expected = rendered(&mut reference, &reference_output);
    recovery_integration_admit(&mut worker, 92, 6, target, 1000, 40);
    worker.event(92, EventKind::TextPage(candidate), &output);
    recovery_integration_assert_display(&mut worker, &output, &previous, origin, old_range);

    for late in [
        EventKind::TextPage(recovery_integration_page(&source, 4000)),
        EventKind::Failed(messages::Failure {
            code: 1,
            message: "owned stale failure beside a current request".into(),
        }),
        EventKind::Cancelled,
        EventKind::Completed,
    ] {
        worker.event(91, late, &output);
        assert_eq!(worker.viewport_request, Some(92));
        assert!(worker.positions.has_pending());
        recovery_integration_assert_display(&mut worker, &output, &previous, origin, old_range);
    }
    worker.event(92, EventKind::Completed, &output);
    let completed = rendered(&mut worker, &output);
    assert_eq!(completed.generation, previous.generation);
    assert_eq!(completed.position_sequence[0], 6);
    assert_eq!(completed.position as f32, target as f32);
    assert!((worker.origin[1] - wanted_y).abs() < 0.001);
    assert_eq!(
        worker.page.as_ref().map(|page| (page.start, page.end)),
        Some((1000, 1080))
    );
    assert!(completed.accessible_text.as_bytes() == &source.as_bytes()[1000..1080]);
    assert!(
        completed.pixels == expected.pixels,
        "completed candidate pixels must match the real CPU camera"
    );
    assert!(worker.positions.source.is_none());
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
}

#[test]
fn scrollbar_source_recovery_active_failure_preserves_the_displayed_page_and_camera() {
    for terminal in [
        EventKind::Completed,
        EventKind::Cancelled,
        EventKind::Failed(messages::Failure {
            code: 1,
            message: "owned active request failure".into(),
        }),
    ] {
        let (mut worker, output, source, previous) = recovery_integration_worker();
        let origin = worker.origin;
        let old_range = {
            let page = worker.page.as_ref().unwrap();
            (page.start, page.end)
        };
        recovery_integration_admit(&mut worker, 91, 4, 0.5, 4000, 512);
        worker.event(
            91,
            EventKind::TextPage(recovery_integration_page(&source, 4000)),
            &output,
        );
        recovery_integration_assert_display(&mut worker, &output, &previous, origin, old_range);
        worker.event(91, terminal, &output);
        let (failed, unavailable) = recovery_v2_observed_frame(&mut worker, &output);
        assert_eq!(failed.generation, previous.generation);
        assert_eq!(failed.position_sequence, previous.position_sequence);
        assert_eq!(unavailable, vec![(previous.generation, 4)]);
        assert_eq!(worker.origin, origin);
        assert_eq!(
            worker.page.as_ref().map(|page| (page.start, page.end)),
            Some(old_range)
        );
        assert!(failed.accessible_text == previous.accessible_text);
        assert!(
            failed.pixels == previous.pixels,
            "a rejected candidate changed displayed pixels"
        );
        assert!(worker.positions.source.is_none());
        assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    }
}

#[test]
fn scrollbar_source_recovery_byte_cropped_reread_cannot_commit_an_uncovered_candidate() {
    use crate::position_ack::{MissingEdge, SourceRecovery, SourceWindow};

    let (mut worker, output, source, previous) = recovery_integration_worker();
    let origin = worker.origin;
    let old_range = {
        let page = worker.page.as_ref().unwrap();
        (page.start, page.end)
    };
    let candidate = recovery_integration_page(&source, 4000);
    let end = candidate.source.end().unwrap();
    let mut recovery = SourceRecovery::new(messages::Viewport {
        source_anchor: candidate.source.offset,
        backwards: false,
        max_bytes: candidate.text.len() as u32,
        max_lines: 40,
    });
    let reread = recovery
        .reread(
            SourceWindow {
                start: candidate.source.offset,
                end,
                before: candidate.has_before,
                after: candidate.has_after,
            },
            MissingEdge::After,
        )
        .expect("the first completed miss can reserve a larger line window");
    assert_eq!(reread.max_bytes as usize, candidate.text.len());
    assert_eq!(reread.max_lines, 80);

    // Model admission of that second request. Its unchanged byte budget can
    // legitimately return the same real crop despite the larger line budget.
    recovery_integration_admit(
        &mut worker,
        92,
        4,
        0.5,
        reread.source_anchor,
        reread.max_lines,
    );
    worker.positions.source = Some(recovery);
    worker.event(92, EventKind::TextPage(candidate), &output);
    recovery_integration_assert_display(&mut worker, &output, &previous, origin, old_range);
    worker.event(92, EventKind::Completed, &output);
    let (failed, unavailable) = recovery_v2_observed_frame(&mut worker, &output);
    assert_eq!(failed.generation, previous.generation);
    assert_eq!(failed.position_sequence, previous.position_sequence);
    assert_eq!(unavailable, vec![(previous.generation, 4)]);
    assert_eq!(worker.origin, origin);
    assert_eq!(
        worker.page.as_ref().map(|page| (page.start, page.end)),
        Some(old_range)
    );
    assert!(failed.accessible_text == previous.accessible_text);
    assert!(
        failed.pixels == previous.pixels,
        "a byte-cropped miss changed displayed pixels"
    );
    assert!(worker.positions.source.is_none());
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
}

fn recovery_v2_ascii_page(source: &str, start: usize, end: usize) -> viewer::TextPage {
    assert!(source.is_ascii() && start < end && end <= source.len());
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
        partial_line: (start > 0 && source.as_bytes()[start - 1] != b'\n')
            || (end < source.len() && !text.ends_with('\n')),
        text,
        spans: Vec::new(),
        has_before: start > 0,
        has_after: end < source.len(),
    };
    page.validate().unwrap();
    page
}

fn recovery_v2_byte_worker() -> (Worker, Mailbox, String, ViewerFrame) {
    let mut source = format!("{}\n{}", "H".repeat(128), "p\n".repeat(2000));
    source.push_str(&"T".repeat(20_000 - source.len()));
    assert_eq!(source.len(), 20_000);
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    worker.generation = 1;
    worker.positions.reset().unwrap();
    worker.length = source.len() as u64;
    worker.geometry = ViewerGeometry {
        width: 350,
        height: 110,
        scale: 1.0,
    };
    worker.raster.enabled = false;
    assert!(worker.set_page(
        Page::text(
            recovery_v2_ascii_page(&source, 129, 209),
            &worker.appearance
        ),
        &output,
    ));
    let generation = worker.positions.generation;
    let position = position_at(&worker, 31.5);
    worker.input(
        ViewerInput::Position {
            generation,
            sequence: 3,
            horizontal: false,
            position,
        },
        &output,
    );
    let first = rendered(&mut worker, &output);
    assert_eq!(first.position_sequence[0], 3);
    assert!((worker.origin[1] - 31.5).abs() < 0.001);
    (worker, output, source, first)
}

fn recovery_v2_admit_byte_page(
    worker: &mut Worker,
    generation: u64,
    sequence: u64,
    position: f64,
    page: &viewer::TextPage,
) {
    assert_eq!(generation, worker.positions.generation);
    recovery_integration_admit(worker, 91, sequence, position, page.source.offset, 512);
    worker.positions.source = Some(crate::position_ack::SourceRecovery::new(
        messages::Viewport {
            source_anchor: if position == 1.0 {
                page.source.end().unwrap()
            } else {
                page.source.offset
            },
            backwards: position == 1.0,
            max_bytes: page.text.len() as u32,
            max_lines: 512,
        },
    ));
}

#[test]
fn scrollbar_source_recovery_completed_one_line_byte_page_uses_the_byte_inverse() {
    let (mut worker, output, source, previous) = recovery_v2_byte_worker();
    let origin = worker.origin;
    let page = recovery_v2_ascii_page(&source, 5000, 5064);
    // A short unwrapped fragment shows all 64 bytes. Its camera starts at byte
    // 5000 within the remaining 19,936-byte scroll range.
    let target = 5000.0 / 19_936.0;
    recovery_v2_admit_byte_page(&mut worker, previous.generation, 4, target, &page);
    worker.event(91, EventKind::TextPage(page), &output);
    recovery_integration_assert_display(&mut worker, &output, &previous, origin, (129, 209));
    worker.event(91, EventKind::Completed, &output);
    let completed = rendered(&mut worker, &output);
    assert_eq!(completed.generation, previous.generation);
    assert_eq!(completed.position_sequence, [4, 0]);
    assert_eq!(completed.position as f32, target as f32);
    assert_eq!(worker.origin, [0.0, 0.0]);
    assert_eq!(
        worker.page.as_ref().map(|page| (page.start, page.end)),
        Some((5000, 5064))
    );
    assert!(completed.accessible_text.as_bytes() == &source.as_bytes()[5000..5064]);
    assert!(worker.shaped.as_ref().unwrap().height() < worker.geometry.height as f32);
    assert!(
        worker.scroll_map.is_none(),
        "this is the explicit byte fallback"
    );
    assert!(worker.positions.source.is_none());
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
}

#[test]
fn scrollbar_source_recovery_near_home_and_end_byte_crops_cannot_acknowledge_file_edges() {
    for (target, start, end) in [(0.0, 2, 66), (1.0, 19_934, 19_998)] {
        let (mut worker, output, source, previous) = recovery_v2_byte_worker();
        let origin = worker.origin;
        let page = recovery_v2_ascii_page(&source, start, end);
        assert!(page.has_before && page.has_after);
        assert!(!page.text.contains('\n'));
        recovery_v2_admit_byte_page(&mut worker, previous.generation, 4, target, &page);
        worker.event(91, EventKind::TextPage(page), &output);
        recovery_integration_assert_display(&mut worker, &output, &previous, origin, (129, 209));
        worker.event(91, EventKind::Completed, &output);
        let (refused, unavailable) = recovery_v2_observed_frame(&mut worker, &output);
        assert_eq!(refused.generation, previous.generation);
        assert_eq!(
            refused.position_sequence, previous.position_sequence,
            "two clipped bytes are still outside the real file edge"
        );
        assert_eq!(unavailable, vec![(previous.generation, 4)]);
        assert_eq!(worker.origin, origin);
        assert_eq!(
            worker.page.as_ref().map(|page| (page.start, page.end)),
            Some((129, 209))
        );
        assert!(refused.accessible_text == previous.accessible_text);
        assert!(
            refused.pixels == previous.pixels,
            "a clipped endpoint replaced the displayed pixels"
        );
        assert!(worker.positions.source.is_none());
        assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    }
}

#[test]
fn scrollbar_source_recovery_real_home_and_end_byte_pages_acknowledge_the_actual_edges() {
    for (target, start, end) in [(0.0, 0, 64), (1.0, 19_936, 20_000)] {
        let (mut worker, output, source, previous) = recovery_v2_byte_worker();
        let origin = worker.origin;
        let page = recovery_v2_ascii_page(&source, start, end);
        assert_eq!(
            (page.has_before, page.has_after),
            (target != 0.0, target != 1.0)
        );
        assert!(!page.text.contains('\n'));
        recovery_v2_admit_byte_page(&mut worker, previous.generation, 4, target, &page);
        worker.event(91, EventKind::TextPage(page), &output);
        recovery_integration_assert_display(&mut worker, &output, &previous, origin, (129, 209));
        worker.event(91, EventKind::Completed, &output);
        let completed = rendered(&mut worker, &output);
        assert_eq!(completed.generation, previous.generation);
        assert_eq!(completed.position_sequence, [4, 0]);
        assert_eq!(completed.position, target);
        assert_eq!(worker.origin, [0.0, 0.0]);
        assert_eq!(
            worker.page.as_ref().map(|page| (page.start, page.end)),
            Some((start as u64, end as u64))
        );
        assert!(completed.accessible_text.as_bytes() == &source.as_bytes()[start..end]);
        assert!(worker.shaped.as_ref().unwrap().height() < worker.geometry.height as f32);
        assert!(worker.scroll_map.is_none());
        assert!(worker.positions.source.is_none());
        assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    }
}

fn recovery_v2_observed_frame(
    worker: &mut Worker,
    output: &Mailbox,
) -> (ViewerFrame, Vec<(u64, u64)>) {
    worker.paint(output).unwrap();
    let mut frame = None;
    let mut unavailable = Vec::new();
    for message in output.take() {
        match message {
            ViewerOutput::Frame(current) => {
                assert!(frame.is_none(), "one actual frame per terminal observation");
                frame = Some(*current);
            }
            ViewerOutput::VerticalPositionUnavailable {
                generation,
                sequence,
            } => unavailable.push((generation, sequence)),
            _ => {}
        }
    }
    (frame.expect("actual painted frame"), unavailable)
}

fn recovery_v2_retained_completion(stale_wheel: bool) {
    use crate::position_ack::{MissingEdge, SourceRecovery, SourceWindow};

    let source = mapped_prefix_source(false);
    let first_page = mapped_prefix_page(&source, 4000);
    let first_end = first_page.source.end().unwrap();
    let (mut worker, output) = mapped_prefix_worker(first_page.clone(), source.len() as u64);
    worker.input(ViewerInput::RetainedRaster(true), &output);
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
    let target = (4000.0 + 35.5) / (20_000.0 - 306.0);
    let maximum = (crate::viewport::available_height(
        worker.page.as_ref().unwrap(),
        worker.shaped.as_ref().unwrap(),
        worker.geometry.height as f32,
    ) - worker.geometry.height as f32)
        .max(0.0);
    let high = worker
        .scroll_map
        .as_ref()
        .unwrap()
        .metrics(f64::from(maximum), f64::from(worker.geometry.height))
        .unwrap()
        .0;
    assert!(
        target > high,
        "the actual eight-line map must miss the target"
    );

    // The real displayed map supplies the admitted first miss. Model admission
    // of its larger provider request while preserving SourceRecovery history.
    let mut recovery = SourceRecovery::new(messages::Viewport {
        source_anchor: first_page.source.offset,
        backwards: false,
        max_bytes: 12_288,
        max_lines: 8,
    });
    let reread = recovery
        .reread(
            SourceWindow {
                start: first_page.source.offset,
                end: first_end,
                before: first_page.has_before,
                after: first_page.has_after,
            },
            MissingEdge::After,
        )
        .expect("the completed first miss admits a larger Source line window");
    assert_eq!((reread.source_anchor, reread.backwards), (4000, false));
    assert_eq!(reread.max_lines, 16);
    let start = reread.source_anchor as usize;
    let length: usize = source[start..]
        .split_inclusive('\n')
        .take(reread.max_lines as usize)
        .map(str::len)
        .sum();
    let corrected = recovery_v2_ascii_page(&source, start, start + length);
    assert!(corrected.source.end().unwrap() > first_end);
    assert_eq!(
        corrected.text.bytes().filter(|&byte| byte == b'\n').count(),
        16
    );
    recovery_integration_admit(
        &mut worker,
        92,
        4,
        target,
        reread.source_anchor,
        reread.max_lines,
    );
    worker.positions.source = Some(recovery);
    worker.event(92, EventKind::TextPage(corrected), &output);
    assert!(worker.positions.has_pending() && worker.positions.source.is_some());
    assert_eq!(worker.positions.acknowledged, first.position_sequence);
    worker.paint(&output).unwrap();
    assert!(
        !output
            .take()
            .iter()
            .any(|message| matches!(message, ViewerOutput::Frame(_))),
        "the admitted recovery data cannot publish acknowledged pixels before Completed"
    );
    if stale_wheel {
        // This legacy motion must be retired when the absolute Source camera completes.
        worker
            .motion
            .defer(MotionOwner::Viewport(92), ScrollStep::new(9.0, 17.25));
    }
    worker.event(92, EventKind::Completed, &output);
    let (completed, unavailable) = recovery_v2_observed_frame(&mut worker, &output);
    assert!(unavailable.is_empty());
    assert_eq!(completed.generation, first.generation);
    assert_eq!(completed.position_sequence, [4, 0]);
    assert!(worker.motion.is_idle());
    assert_eq!(completed.position, worker.position_metrics().unwrap().0);
    assert_eq!(completed.position as f32, target as f32);
    assert!(worker.origin[1] > 0.0);
    assert_eq!(worker.text.coordinate.unwrap().line, 2001);
    assert!(worker.positions.source.is_none());
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    // The exact recovered camera maps to the target; the window then rests on the nearest
    // whole pixel, where the strips' pixels are exact.
    let recovered = completed.raster.clone().unwrap();
    let completed = settle_whole(&mut worker, &output, completed);
    let visible = corrected_source_visible_pixels(&completed);
    assert!(
        visible != first_visible,
        "the recovered camera must replace the missed camera"
    );
    let info = &recovered;
    assert_eq!(
        info.viewport,
        [worker.geometry.width, worker.geometry.height]
    );
    let mapped = info
        .scroll_map
        .as_ref()
        .expect("the retained recovery carries its actual map")
        .metrics(info.origin[1], f64::from(info.viewport[1]))
        .unwrap()
        .0;
    assert_eq!(mapped as f32, target as f32);
    let gutter_bytes = info.bands.as_ref().unwrap().gutter_width as usize * 4;
    assert!(
        visible
            .chunks_exact(worker.geometry.width as usize * 4)
            .any(|row| {
                row[..gutter_bytes]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel.as_slice() != &row[..4])
            }),
        "the recovered visible gutter must contain actual numbered glyphs"
    );
    let origin = worker.origin;
    worker.input(ViewerInput::RetainedRaster(false), &output);
    assert_eq!(worker.origin, origin);
    let reference = rendered(&mut worker, &output);
    assert!(reference.raster.is_none());
    assert_eq!(reference.generation, first.generation);
    assert_eq!(reference.position_sequence, [4, 0]);
    // The reference shows the camera the window settled on (`settle_whole`).
    assert_eq!(reference.position, completed.position);
    assert_eq!(visible.len(), reference.pixels.len());
    if let Some(at) = visible
        .iter()
        .zip(reference.pixels.iter())
        .position(|(actual, expected)| actual != expected)
    {
        let width = worker.geometry.width as usize;
        let pixel = at / 4;
        let (x, y, channel) = (pixel % width, pixel / width, at % 4);
        let pixel_start = at - channel;
        let gutter_width = gutter_bytes / 4;
        let mut differing_channel_bytes = [0usize; 2];
        for (offset, (actual, expected)) in visible.iter().zip(reference.pixels.iter()).enumerate()
        {
            if actual != expected {
                differing_channel_bytes[usize::from(offset / 4 % width >= gutter_width)] += 1;
            }
        }
        let before_y = (info.origin[1] - info.raster_origin[1]).round() as f32;
        let retained_local_y = origin[1] - before_y;
        eprintln!(
            "SourceRecovery pixel diagnostic stale_wheel={stale_wheel} viewport={:?} first=({x},{y},{channel}) retained_rgba={:?} cpu_rgba={:?} differing_channel_bytes_gutter_body={:?} gutter_width={gutter_width} cpu_origin={origin:?} retained_origin={:?} raster_origin={:?} before_y={before_y} retained_local_y={retained_local_y}",
            [worker.geometry.width, worker.geometry.height],
            &visible[pixel_start..pixel_start + 4],
            &reference.pixels[pixel_start..pixel_start + 4],
            differing_channel_bytes,
            info.origin,
            info.raster_origin,
        );
        let annotation_phase_bits = |offset: f32| {
            let offset = f64::from(offset);
            ((offset - offset.floor()) as f32).to_bits()
        };
        if let Some(lines) = &worker.text.gutter {
            let reach = f64::from(worker.appearance.font_size * worker.geometry.scale * 2.0);
            for line in lines
                .iter()
                .filter(|line| (f64::from(line.y) - f64::from(origin[1]) - y as f64).abs() <= reach)
                .take(4)
            {
                let cpu_offset = origin[1] - line.y;
                let retained_offset = retained_local_y - line.y;
                let translation_error =
                    f64::from(cpu_offset) - f64::from(retained_offset) - f64::from(before_y);
                eprintln!(
                    "SourceRecovery annotation diagnostic line={} line_y={} cpu_offset={cpu_offset} retained_offset={retained_offset} annotation_origin_phase_bits_cpu_retained={:?} translation_error={translation_error} (annotation origins, not actual glyph phases)",
                    line.line,
                    line.y,
                    [
                        annotation_phase_bits(cpu_offset),
                        annotation_phase_bits(retained_offset)
                    ],
                );
            }
        }
    }
    assert_eq!(
        visible
            .iter()
            .zip(reference.pixels.iter())
            .position(|(actual, expected)| actual != expected),
        None,
        "SourceRecovery retained body and numbered gutter must match the CPU camera"
    );
}

#[test]
fn scrollbar_source_recovery_retained_completion_keeps_the_exact_numbered_cpu_viewport() {
    recovery_v2_retained_completion(false);
}

#[test]
fn scrollbar_source_recovery_completion_retires_stale_queued_wheel_deltas() {
    recovery_v2_retained_completion(true);
}

#[test]
fn scrollbar_source_recovery_unavailable_old_seek_preserves_the_newer_input_queue_cell() {
    use crate::input_queue::{Message, PositionQueue};
    use std::sync::mpsc;

    let (mut worker, output, source, previous) = recovery_v2_byte_worker();
    worker.generation = 41;
    assert_ne!(worker.generation, previous.generation);
    let origin = worker.origin;
    let candidate = recovery_v2_ascii_page(&source, 5000, 5064);
    let queue = PositionQueue::default();
    let (sender, receiver) = mpsc::sync_channel(1);
    assert!(queue.enqueue(
        ViewerInput::Position {
            generation: previous.generation,
            sequence: 4,
            horizontal: false,
            position: 0.5,
        },
        |message| sender.try_send(message).is_ok(),
    ));
    let Some(ViewerInput::Position {
        generation,
        sequence,
        horizontal: false,
        position,
    }) = receiver.try_recv().unwrap().into_input()
    else {
        panic!("the first tagged position must be consumed from its real queue cell");
    };
    recovery_v2_admit_byte_page(&mut worker, generation, sequence, position, &candidate);
    worker.event(91, EventKind::TextPage(candidate), &output);
    recovery_integration_assert_display(&mut worker, &output, &previous, origin, (129, 209));

    let newest_y = 92.25;
    let target = position_at(&worker, newest_y);
    assert!(queue.enqueue(
        ViewerInput::Position {
            generation: previous.generation,
            sequence: 5,
            horizontal: false,
            position: target,
        },
        |message| sender.try_send(message).is_ok(),
    ));
    let queued = receiver
        .try_recv()
        .expect("the real queue accepted the newer seek");
    assert!(matches!(&queued, Message::Position(_)));
    assert!(
        !worker.positions.has_deferred(),
        "the worker has not drained the newer cell"
    );
    worker.event(91, EventKind::Completed, &output);
    let (unavailable_frame, unavailable) = recovery_v2_observed_frame(&mut worker, &output);
    assert_eq!(unavailable, vec![(previous.generation, 4)]);
    assert_eq!(
        worker.generation, 41,
        "the plugin session generation remains separate"
    );
    assert_eq!(unavailable_frame.generation, previous.generation);
    assert_eq!(
        unavailable_frame.position_sequence,
        previous.position_sequence
    );
    assert_eq!(worker.origin, origin);
    assert_eq!(
        worker.page.as_ref().map(|page| (page.start, page.end)),
        Some((129, 209))
    );
    assert!(unavailable_frame.pixels == previous.pixels);
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    assert!(receiver.try_recv().is_err());

    // Consuming Message seals the cell only now, after the older completion.
    let newest = queued
        .into_input()
        .expect("the newer accepted cell survived old failure");
    assert!(matches!(
        &newest,
        ViewerInput::Position {
            generation,
            sequence: 5,
            horizontal: false,
            position,
        } if *generation == previous.generation && *position == target
    ));
    worker.input(newest, &output);
    let (completed, unavailable) = recovery_v2_observed_frame(&mut worker, &output);
    assert!(unavailable.is_empty());
    assert_eq!(completed.generation, previous.generation);
    assert_eq!(completed.position_sequence, [5, 0]);
    assert_eq!(worker.generation, 41);
    assert_eq!(completed.position as f32, target as f32);
    assert!((worker.origin[1] - newest_y).abs() < 0.001);
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());

    let (mut reference, reference_output, _, _) = recovery_v2_byte_worker();
    reference.origin[1] = newest_y;
    reference.frames.invalidate();
    reference.dirty = true;
    let reference = rendered(&mut reference, &reference_output);
    assert!(
        completed.pixels == reference.pixels,
        "the queue race changed the latest CPU camera"
    );
}

#[test]
fn scrollbar_source_recovery_later_wheel_retires_only_the_active_absolute_seek() {
    use crate::input_queue::PositionQueue;
    use std::sync::mpsc;

    let (mut worker, output, source, previous) = recovery_v2_byte_worker();
    let candidate = recovery_v2_ascii_page(&source, 5000, 5064);
    recovery_v2_admit_byte_page(&mut worker, previous.generation, 4, 0.5, &candidate);
    worker.event(91, EventKind::TextPage(candidate.clone()), &output);
    worker.input(ViewerInput::Scroll { x: 0.0, y: 5.0 }, &output);
    let (wheel, unavailable) = recovery_v2_observed_frame(&mut worker, &output);
    assert_eq!(unavailable, vec![(previous.generation, 4)]);
    assert_eq!(wheel.generation, previous.generation);
    assert_eq!(wheel.position_sequence, previous.position_sequence);
    assert!((worker.origin[1] - 36.5).abs() < 0.001);
    assert!(wheel.accessible_text == previous.accessible_text);
    assert!(worker.positions.source.is_none());
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    let wheel_origin = worker.origin;
    for late in [EventKind::TextPage(candidate), EventKind::Completed] {
        worker.event(91, late, &output);
        assert_eq!(worker.origin, wheel_origin);
        assert_eq!(worker.positions.generation, previous.generation);
        assert_eq!(worker.positions.acknowledged, previous.position_sequence);
        assert_eq!(
            worker.page.as_ref().map(|page| (page.start, page.end)),
            Some((129, 209))
        );
        assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    }

    let newest_y = 92.25;
    let target = position_at(&worker, newest_y);
    let queue = PositionQueue::default();
    let (sender, receiver) = mpsc::sync_channel(1);
    assert!(queue.enqueue(
        ViewerInput::Position {
            generation: previous.generation,
            sequence: 5,
            horizontal: false,
            position: target,
        },
        |message| sender.try_send(message).is_ok(),
    ));
    let newest = receiver.try_recv().unwrap().into_input().unwrap();
    worker.input(newest, &output);
    let (completed, unavailable) = recovery_v2_observed_frame(&mut worker, &output);
    assert!(
        unavailable.is_empty(),
        "late retired callbacks cannot report another outcome"
    );
    assert_eq!(completed.generation, previous.generation);
    assert_eq!(completed.position_sequence, [5, 0]);
    assert_eq!(completed.position as f32, target as f32);
    assert!((worker.origin[1] - newest_y).abs() < 0.001);
    assert!(worker.motion.is_idle());
    assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    let (mut reference, reference_output, _, _) = recovery_v2_byte_worker();
    reference.origin[1] = newest_y;
    reference.frames.invalidate();
    reference.dirty = true;
    let reference = rendered(&mut reference, &reference_output);
    assert!(
        completed.pixels == reference.pixels,
        "the wheel retirement changed the newer CPU camera"
    );
}
