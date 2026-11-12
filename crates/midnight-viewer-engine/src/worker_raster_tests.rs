use super::*;

fn content() -> String {
    (0..120)
        .map(|row| format!("row {row:03}: exact Tokyo 東京 and accented café pixels; abcdefghijklmnopqrstuvwxyz\n"))
        .collect()
}

fn fragment(source: &str, start: usize, end: usize) -> Page {
    let text = &source[start..end];
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
            text: text.into(),
            spans: vec![],
            has_before: start != 0,
            has_after: end != source.len(),
            partial_line: false,
        },
        &worker().0.appearance,
    )
}

fn retained(mode: u16) -> (Worker, Mailbox) {
    let (mut worker, output) = worker();
    worker.mode = mode;
    let text = content();
    worker.length = text.len() as u64;
    worker.input(ViewerInput::RetainedRaster(true), &output);
    assert!(worker.set_page(fragment(&text, 0, text.len()), &output));
    (worker, output)
}

fn paint(worker: &mut Worker, output: &Mailbox) -> ViewerFrame {
    worker.paint(output).unwrap();
    output
        .take()
        .into_iter()
        .find_map(|message| {
            if let ViewerOutput::Frame(frame) = message {
                Some(*frame)
            } else {
                None
            }
        })
        .expect("prepared content frame")
}

fn pan(
    worker: &mut Worker,
    output: &Mailbox,
    info: &ViewerRasterInfo,
    sequence: u64,
    origin: [f64; 2],
    settled: bool,
) {
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::PanTo {
                sequence,
                origin,
                settled,
            },
        },
        output,
    );
}

fn visible(frame: &ViewerFrame) -> Vec<u8> {
    let info = frame.raster.as_ref().expect("retained pixels");
    let gutter = info.gutter.as_ref().map_or(0, |g| g.width);
    let dx = info.origin[0] - info.raster_origin[0];
    let dy = info.origin[1] - info.raster_origin[1];
    assert!(
        (dx - dx.round()).abs() < 1e-5 && (dy - dy.round()).abs() < 1e-5,
        "exact settled raster must permit an integer crop: {dx},{dy}"
    );
    let [width, height] = info.viewport;
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        let source_y = y + dy.round() as usize;
        if let Some(gutter) = &info.gutter {
            let at = source_y * gutter.width as usize * 4;
            pixels.extend_from_slice(&gutter.pixels[at..at + gutter.width as usize * 4]);
        }
        let at = (source_y * frame.width as usize + dx.round() as usize) * 4;
        pixels.extend_from_slice(&frame.pixels[at..at + (width - gutter) as usize * 4]);
    }
    pixels
}

fn assert_pixels_equal(actual: &[u8], expected: &[u8], context: &str) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{context}: pixel buffer lengths"
    );
    if let Some(index) = actual.iter().zip(expected).position(|(a, b)| a != b) {
        let start = index / 4 * 4;
        let max_delta = actual
            .iter()
            .zip(expected)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        panic!(
            "{context}: first different pixel {}, actual {:?}, expected {:?}, max channel delta {max_delta}",
            index / 4,
            &actual[start..start + 4],
            &expected[start..start + 4]
        );
    }
}

#[test]
fn retained_text_source_and_markdown_have_guard_pixels_without_changing_wrap_width() {
    for mode in [
        viewer::MODE_TEXT,
        viewer::MODE_SOURCE,
        viewer::MODE_DOCUMENT,
    ] {
        let (mut worker, output) = retained(mode);
        worker.wrap = true;
        worker.reflow(&output);
        let lines = worker.shaped.as_ref().unwrap().line_count();
        let frame = paint(&mut worker, &output);
        let info = frame
            .raster
            .as_ref()
            .expect("native opt-in needs retained coverage");
        assert!(frame.image.is_none());
        assert_eq!(info.viewport, [240, 100]);
        assert!(info.coverage_max[1] >= info.origin[1] + 256.0);
        assert_eq!(worker.shaped.as_ref().unwrap().line_count(), lines);
        assert_eq!(info.gutter.is_some(), mode == viewer::MODE_SOURCE);
        assert_eq!(info.limit_min[1], Some(0.0));
        assert!(info.limit_max[1].unwrap() > info.coverage_max[1]);
        assert!(frame.pixels.len() <= 6 * 1024 * 1024 * 4);
    }
}

#[test]
fn retained_pan_acknowledges_fractional_origin_without_allocating_or_rasterizing_again() {
    let (mut worker, output) = retained(viewer::MODE_TEXT);
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    for (sequence, y) in [(1, 7.25), (2, 17.5), (3, 4.125)] {
        pan(&mut worker, &output, info, sequence, [0.0, y], false);
        let frame = paint(&mut worker, &output);
        let next = frame.raster.as_ref().unwrap();
        assert_eq!(next.surface, info.surface);
        assert_eq!(
            next.pixel_id, info.pixel_id,
            "movement inside guard is metadata only"
        );
        assert!(Arc::ptr_eq(&frame.pixels, &first.pixels));
        assert_eq!(next.origin, [0.0, y]);
        assert_eq!(next.pan_sequence, sequence);
        assert!(!next.settled);
    }
}

#[test]
fn retained_settle_matches_the_exact_viewport_reference_and_pinned_source_gutter() {
    for mode in [
        viewer::MODE_TEXT,
        viewer::MODE_SOURCE,
        viewer::MODE_DOCUMENT,
    ] {
        let (mut worker, output) = retained(mode);
        let first = paint(&mut worker, &output);
        pan(
            &mut worker,
            &output,
            first.raster.as_ref().unwrap(),
            1,
            [13.25, 17.375],
            true,
        );
        let settled = paint(&mut worker, &output);
        let info = settled.raster.as_ref().unwrap();
        assert!(info.settled);
        let origin = worker.origin;
        worker.input(ViewerInput::RetainedRaster(false), &output);
        worker.origin = origin;
        let reference = paint(&mut worker, &output);
        assert!(reference.raster.is_none());
        assert_pixels_equal(
            &visible(&settled),
            &reference.pixels,
            &format!("mode {mode} exact settled pixels"),
        );
    }
}

#[test]
fn retained_stale_sequence_and_replaced_context_cannot_move_the_camera() {
    let (mut worker, output) = retained(viewer::MODE_TEXT);
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    pan(&mut worker, &output, info, 2, [0.0, 17.5], false);
    pan(&mut worker, &output, info, 1, [0.0, 200.0], false);
    let latest = paint(&mut worker, &output);
    assert_eq!(latest.raster.as_ref().unwrap().origin, [0.0, 17.5]);
    worker.input(
        ViewerInput::Resize(ViewerGeometry {
            width: 241,
            height: 100,
            scale: 1.0,
        }),
        &output,
    );
    let resized = paint(&mut worker, &output);
    assert_ne!(resized.raster.as_ref().unwrap().surface, info.surface);
    pan(&mut worker, &output, info, 100, [0.0, 200.0], false);
    let after = paint(&mut worker, &output);
    assert_eq!(
        after.raster.as_ref().unwrap().origin,
        resized.raster.as_ref().unwrap().origin
    );
}

#[test]
fn retained_selection_uses_displayed_camera_instead_of_prefetch_camera() {
    let (mut worker, output) = retained(viewer::MODE_SOURCE);
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    pan(&mut worker, &output, info, 1, [0.0, 128.0], false);
    let displayed = [0.0, 17.375];
    let margin = worker.text_margin();
    let expected = worker
        .shaped
        .as_ref()
        .unwrap()
        .hit_test(40.0, displayed[1] as f32 + 8.0)
        .unwrap();
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Select {
                pixel_id: info.pixel_id,
                origin: displayed,
                x: margin + 40.0,
                y: 8.0,
                extend: false,
            },
        },
        &output,
    );
    assert_eq!(worker.selection, Some((expected, expected)));
    let selected = paint(&mut worker, &output);
    assert_eq!(selected.raster.as_ref().unwrap().origin, displayed);
}

#[test]
fn retained_horizontal_scrollbar_and_scrolled_reflow_publish_new_contexts() {
    let (mut worker, output) = retained(viewer::MODE_SOURCE);
    let first = paint(&mut worker, &output);
    worker.input(
        ViewerInput::Position {
            generation: first.generation,
            sequence: 1,
            horizontal: true,
            position: 0.6,
        },
        &output,
    );
    let horizontal = paint(&mut worker, &output);
    assert!(horizontal.raster.as_ref().unwrap().origin[0] > 0.0);
    assert_ne!(
        horizontal.raster.as_ref().unwrap().surface,
        first.raster.as_ref().unwrap().surface
    );
    drop(first);
    worker.input(ViewerInput::Wrap(true), &output);
    let wrapped = paint(&mut worker, &output);
    assert_eq!(wrapped.raster.as_ref().unwrap().origin[0], 0.0);
    drop(horizontal);
    worker.input(
        ViewerInput::Resize(ViewerGeometry {
            width: 180,
            height: 125,
            scale: 1.25,
        }),
        &output,
    );
    let resized = paint(&mut worker, &output);
    assert_eq!(resized.raster.as_ref().unwrap().viewport, [180, 125]);
}

#[test]
fn retained_page_rebase_keeps_continuous_origin_and_rejects_old_pixel_selection() {
    let (mut worker, output) = retained(viewer::MODE_SOURCE);
    let source = content();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    assert!(worker.set_page(fragment(&source, 0, starts[60]), &output));
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::SelectPage {
                pixel_id: info.pixel_id,
                origin: info.origin,
            },
        },
        &output,
    );
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Copy,
        },
        &output,
    );
    let selected = output
        .take()
        .into_iter()
        .find_map(|event| {
            if let ViewerOutput::CopyText(text) = event {
                Some(text)
            } else {
                None
            }
        })
        .unwrap();
    assert!(
        selected.len() < source[..starts[60]].len(),
        "guard rows are excluded from SelectPage"
    );
    let target = [
        4.25,
        f64::from(worker.shaped.as_ref().unwrap().height() - worker.geometry.height as f32)
            - 32.375,
    ];
    pan(&mut worker, &output, info, 1, target, false);
    assert!(worker.raster.continuing.is_some());
    assert!(worker.set_page(fragment(&source, starts[30], starts[90]), &output));
    worker.apply_raster_target(&output);
    let next = paint(&mut worker, &output);
    let next_info = next.raster.as_ref().unwrap();
    assert_eq!(next_info.surface, info.surface);
    for (actual, expected) in next_info.origin.iter().zip(target) {
        assert!((actual - expected).abs() < 1e-4);
    }
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Copy,
        },
        &output,
    );
    assert!(
        output
            .take()
            .into_iter()
            .any(|event| matches!(event,ViewerOutput::CopyText(text) if text==selected))
    );
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Select {
                pixel_id: info.pixel_id,
                origin: info.origin,
                x: 60.0,
                y: 5.0,
                extend: false,
            },
        },
        &output,
    );
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Copy,
        },
        &output,
    );
    assert!(
        !output
            .take()
            .into_iter()
            .any(|event| matches!(event, ViewerOutput::CopyText(_))),
        "rejected new selection must not copy stale selection"
    );
}

#[test]
fn retained_three_live_snapshots_bound_pending_pans_and_resume_only_latest_target() {
    let (mut worker, output) = retained(viewer::MODE_SOURCE);
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap().clone();
    let gutter_hold = info.gutter.as_ref().unwrap().pixels.clone();
    pan(&mut worker, &output, &info, 1, [0.0, 7.25], true);
    let second = paint(&mut worker, &output);
    pan(&mut worker, &output, &info, 2, [0.0, 17.5], true);
    let third = paint(&mut worker, &output);
    for sequence in 3..=10_000 {
        pan(&mut worker, &output, &info, sequence, [0.0, 21.375], true);
    }
    worker.paint(&output).unwrap();
    assert!(
        output.take().is_empty(),
        "unprepared target cannot acknowledge"
    );
    drop(first);
    drop(info);
    worker.paint(&output).unwrap();
    assert!(
        output.take().is_empty(),
        "gutter alone keeps its allocation slot live"
    );
    drop(gutter_hold);
    let latest = paint(&mut worker, &output);
    assert_eq!(latest.raster.as_ref().unwrap().pan_sequence, 10_000);
    assert_eq!(latest.raster.as_ref().unwrap().origin, [0.0, 21.375]);
    assert_eq!(
        latest.raster.as_ref().unwrap().pixel_id,
        third.raster.as_ref().unwrap().pixel_id + 1
    );
    drop((second, third, latest));
}

#[test]
fn retained_search_highlight_copy_survives_its_new_context() {
    let (mut worker, output) = retained(viewer::MODE_TEXT);
    let source = content();
    let at = source.find("Tokyo").unwrap();
    worker.found = Some(viewer::SourceRange {
        offset: at as u64,
        length: 5,
    });
    assert!(worker.set_page(fragment(&source, 0, source.len()), &output));
    let frame = paint(&mut worker, &output);
    worker.input(
        ViewerInput::Raster {
            surface: frame.raster.as_ref().unwrap().surface,
            input: ViewerRasterInput::Copy,
        },
        &output,
    );
    assert!(
        output
            .take()
            .into_iter()
            .any(|event| matches!(event,ViewerOutput::CopyText(text) if text=="Tokyo"))
    );
}

#[test]
fn retained_short_incomplete_fragment_falls_back_without_losing_source_progress() {
    let (mut worker, output) = retained(viewer::MODE_SOURCE);
    let source = "x".repeat(20_000);
    worker.length = source.len() as u64;
    assert!(worker.set_page(fragment(&source, 0, 4096), &output));
    let frame = paint(&mut worker, &output);
    assert!(frame.raster.is_none());
    assert!(worker.raster.enabled);
    worker.input(ViewerInput::Scroll { x: 0.0, y: 17.25 }, &output);
    assert_eq!(worker.continuation.as_ref().unwrap().anchor, 4096);
    let eof = "tiny EOF";
    worker.length = eof.len() as u64;
    worker.continuation = None;
    assert!(worker.set_page(fragment(eof, 0, eof.len()), &output));
    let last = paint(&mut worker, &output);
    let info = last.raster.as_ref().unwrap();
    assert_eq!(info.coverage_min, info.coverage_max);
    assert_eq!(info.limit_max[1], Some(info.origin[1]));
}

#[test]
fn retained_refill_preserves_logical_lead_on_scaled_monitors() {
    for scale in [1.0_f32, 1.25, 1.5, 2.25] {
        for direction in [-1.0, 1.0] {
            let (mut worker, output) = retained(viewer::MODE_SOURCE);
            worker.geometry = ViewerGeometry {
                width: (240.0 * scale) as u32,
                height: (100.0 * scale) as u32,
                scale,
            };
            worker.reflow(&output);
            worker.origin[1] = 768.0 * scale;
            worker.reset_raster_context();
            let first = paint(&mut worker, &output);
            let info = first.raster.as_ref().unwrap();
            let edge = if direction > 0.0 {
                info.coverage_max[1]
            } else {
                info.coverage_min[1]
            };
            let origin = [0.0, edge - direction * 48.0 * f64::from(scale)];
            pan(&mut worker, &output, info, 1, origin, false);
            let refreshed = paint(&mut worker, &output);
            let next = refreshed.raster.as_ref().unwrap();
            assert_eq!(next.surface, info.surface);
            assert_ne!(
                next.pixel_id, info.pixel_id,
                "scale {scale}, direction {direction}: refill must start before the last 48 logical pixels"
            );
            assert!((next.origin[1] - origin[1]).abs() < 0.001);
            assert!(if direction > 0.0 {
                next.coverage_max[1] > info.coverage_max[1]
            } else {
                next.coverage_min[1] < info.coverage_min[1]
            });
            assert!(worker.viewport_request.is_none());
            assert!(refreshed.pixels.len() <= 6 * 1024 * 1024 * 4);
        }
    }
}

#[test]
fn retained_refill_budget_limited_guard_does_not_replace_stationary_pixels() {
    let (mut worker, output) = retained(viewer::MODE_SOURCE);
    worker.geometry = ViewerGeometry {
        width: 2048,
        height: 2048,
        scale: 8.0,
    };
    worker.reflow(&output);
    worker.origin = [768.0, 6144.0];
    worker.reset_raster_context();
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    assert!(info.coverage_max[1] - info.origin[1] < 64.0 * 8.0);
    pan(&mut worker, &output, info, 1, info.origin, false);
    let unchanged = paint(&mut worker, &output);
    assert_eq!(unchanged.raster.as_ref().unwrap().pixel_id, info.pixel_id);
    assert!(Arc::ptr_eq(&first.pixels, &unchanged.pixels));
    assert!(unchanged.pixels.len() <= 6 * 1024 * 1024 * 4);
}

#[test]
fn retained_guard_preserves_logical_lookahead_on_scaled_monitors() {
    for scale in [1.0_f32, 1.25, 1.5, 2.25] {
        let geometry = ViewerGeometry {
            width: (1000.0 * scale) as u32,
            height: (600.0 * scale) as u32,
            scale,
        };
        let [x, y] = super::super::raster::guard(geometry, false);
        assert_eq!(x, 0);
        assert_eq!(y, (256.0 * scale).ceil() as u32);
        let requested = ViewerGeometry {
            height: geometry.height + 2 * y,
            ..geometry
        };
        assert_eq!(crate::viewport::line_budget(requested, 20.0 * scale), 58);
    }
}
#[test]
fn retained_guard_geometry_caps_body_and_gutter_at_high_dpi() {
    for (width, height) in [(240, 100), (2250, 1400), (4096, 4096), (1, 4096)] {
        let geometry = bounded_geometry(ViewerGeometry {
            width,
            height,
            scale: 2.25,
        });
        for horizontal in [false, true] {
            let [x, y] = super::super::raster::guard(geometry, horizontal);
            let maximum = (256.0 * geometry.scale).ceil() as u32;
            assert!(x <= maximum && y <= maximum);
            assert!(
                (u64::from(geometry.width) + 2 * u64::from(x))
                    * (u64::from(geometry.height) + 2 * u64::from(y))
                    <= 6 * 1024 * 1024
            );
            let demand = ViewerGeometry {
                height: geometry.height + 2 * y,
                ..geometry
            };
            assert!(crate::viewport::line_budget(demand, 1.0) <= 512);
        }
    }
}

#[test]
fn retained_binary_keeps_fixed_hex_columns_and_exact_settled_pixels() {
    let (mut worker, output) = worker();
    worker.input(ViewerInput::RetainedRaster(true), &output);
    worker.binary = true;
    worker.mode = viewer::MODE_BINARY;
    worker.length = 4096;
    let bytes: Vec<u8> = (0..4096).map(|index| index as u8).collect();
    assert!(worker.set_page(
        Page::binary(
            viewer::BinaryPage {
                revision: 1,
                source: viewer::SourceRange {
                    offset: 0,
                    length: 4096
                },
                has_before: false,
                has_after: false,
            },
            &bytes
        ),
        &output
    ));
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    assert!(info.coverage_max[0] > 0.0 && info.coverage_max[1] >= 256.0);
    assert!(
        first
            .accessible_text
            .starts_with("0000000000000000  00 01 02")
    );
    pan(&mut worker, &output, info, 1, [17.25, 21.375], true);
    let settled = paint(&mut worker, &output);
    worker.input(ViewerInput::RetainedRaster(false), &output);
    let reference = paint(&mut worker, &output);
    assert_pixels_equal(
        &visible(&settled),
        &reference.pixels,
        "binary exact settled pixels",
    );
}

#[test]
fn retained_no_match_context_and_reflow_keep_the_existing_selection_copyable() {
    let (mut worker, output) = retained(viewer::MODE_TEXT);
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::SelectPage {
                pixel_id: info.pixel_id,
                origin: info.origin,
            },
        },
        &output,
    );
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Copy,
        },
        &output,
    );
    let expected = output
        .take()
        .into_iter()
        .find_map(|event| {
            if let ViewerOutput::CopyText(text) = event {
                Some(text)
            } else {
                None
            }
        })
        .unwrap();
    drop(first);
    for reflow in [false, true] {
        if reflow {
            worker.wrap = true;
            worker.reflow(&output);
        } else {
            worker.position_context(&output);
        }
        let frame = paint(&mut worker, &output);
        worker.input(
            ViewerInput::Raster {
                surface: frame.raster.as_ref().unwrap().surface,
                input: ViewerRasterInput::Copy,
            },
            &output,
        );
        assert!(
            output
                .take()
                .into_iter()
                .any(|event| matches!(event,ViewerOutput::CopyText(text) if text==expected))
        );
    }
}

#[test]
fn retained_refill_clamps_narrower_fragment_and_eof_without_rebasing_real_limits() {
    let (mut worker, output) = retained(viewer::MODE_TEXT);
    let source = format!(
        "{}{}",
        "long column abcdefghijklmnopqrstuvwxyz\n".repeat(30),
        "short\n".repeat(35)
    );
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    worker.length = source.len() as u64;
    let full = worker
        .layout_page(&fragment(&source, 0, source.len()))
        .unwrap();
    let true_end = f64::from(full.height() - worker.geometry.height as f32);
    assert!(worker.set_page(fragment(&source, 0, starts[60]), &output));
    let first = paint(&mut worker, &output);
    let target = [
        50.0,
        f64::from(worker.shaped.as_ref().unwrap().height()) + 200.0,
    ];
    pan(
        &mut worker,
        &output,
        first.raster.as_ref().unwrap(),
        1,
        target,
        false,
    );
    assert!(worker.set_page(fragment(&source, starts[30], source.len()), &output));
    worker.apply_raster_target(&output);
    let last = paint(&mut worker, &output);
    let info = last.raster.as_ref().unwrap();
    assert_eq!(info.origin[0], 0.0);
    assert_eq!(info.limit_max[0], Some(0.0));
    assert!(
        (info.limit_max[1].unwrap() - true_end).abs() < 0.01,
        "true EOF must not move to the requested overshoot"
    );
    assert!((info.origin[1] - true_end).abs() < 0.01);
}

#[test]
fn retained_new_page_revokes_old_selection_authority_before_replacement_pixels_exist() {
    let (mut worker, output) = retained(viewer::MODE_SOURCE);
    let source = content();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    assert!(worker.set_page(fragment(&source, 0, starts[60]), &output));
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    let margin = worker.text_margin();
    for (x, extend) in [(margin + 8.0, false), (margin + 80.0, true)] {
        worker.input(
            ViewerInput::Raster {
                surface: info.surface,
                input: ViewerRasterInput::Select {
                    pixel_id: info.pixel_id,
                    origin: info.origin,
                    x,
                    y: 8.0,
                    extend,
                },
            },
            &output,
        );
    }
    assert!(
        worker.selection.is_some_and(|(a, b)| a != b),
        "same pixels stay valid during selection-only repaint"
    );
    let target = [
        0.0,
        f64::from(worker.shaped.as_ref().unwrap().height() - worker.geometry.height as f32) - 32.0,
    ];
    pan(&mut worker, &output, info, 1, target, false);
    assert!(worker.set_page(fragment(&source, starts[30], starts[90]), &output));
    let selection = worker.selection;
    // No paint occurred: the previous pixel ID still exists in the old cache.
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Select {
                pixel_id: info.pixel_id,
                origin: info.origin,
                x: margin + 20.0,
                y: 8.0,
                extend: false,
            },
        },
        &output,
    );
    assert_eq!(
        worker.selection, selection,
        "never hit-test the new page using old pixels"
    );
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Copy,
        },
        &output,
    );
    assert!(
        !output
            .take()
            .into_iter()
            .any(|event| matches!(event, ViewerOutput::CopyText(_)))
    );
}

#[test]
fn retained_semantic_markdown_keeps_heading_code_quote_and_selection_pixels_exact() {
    use midnight_viewer_render::{DocumentBlock, DocumentKind};
    for scale in [1.0, 2.25] {
        let (mut worker, output) = worker();
        worker.mode = viewer::MODE_DOCUMENT;
        worker.geometry = ViewerGeometry {
            width: 420,
            height: 200,
            scale,
        };
        let sections = [
            (DocumentKind::Heading(1), "Retained semantic heading"),
            (
                DocumentKind::Paragraph,
                "A paragraph wraps with accented café and Japanese 東京 text across several visible lines. A longer sentence keeps the visible wrapping width authoritative.",
            ),
            (
                DocumentKind::Code,
                "let accented = \"café\";\nlet guard = 256;\nreturn guard;",
            ),
            (
                DocumentKind::Quote,
                "A quoted paragraph carries its own border and indentation while retained pixels move.",
            ),
            (
                DocumentKind::Paragraph,
                "A final paragraph leaves enough scrollable content for a fractional settled viewport.",
            ),
        ];
        let mut source = String::new();
        let mut blocks = Vec::new();
        for (kind, text) in sections {
            if !source.is_empty() {
                source.push_str("\n\n");
            }
            let start = source.len();
            source.push_str(text);
            blocks.push(DocumentBlock {
                range: start..source.len(),
                kind,
                depth: 0,
            });
        }
        let mut page = fragment(&source, 0, source.len());
        page.document_blocks = blocks;
        worker.length = source.len() as u64;
        worker.input(ViewerInput::RetainedRaster(true), &output);
        assert!(worker.set_page(page, &output));
        worker.selection = Some((0, source.find("let accented").unwrap()));
        let first = paint(&mut worker, &output);
        pan(
            &mut worker,
            &output,
            first.raster.as_ref().unwrap(),
            1,
            [0.0, 17.375],
            true,
        );
        let settled = paint(&mut worker, &output);
        worker.input(ViewerInput::RetainedRaster(false), &output);
        let reference = paint(&mut worker, &output);
        assert_pixels_equal(
            &visible(&settled),
            &reference.pixels,
            &format!("semantic Markdown scale{scale}"),
        );
    }
}

#[test]
fn retained_failed_cancelled_or_empty_guard_reply_allows_a_later_reversal_without_false_ack() {
    for terminal in [
        EventKind::Failed(messages::Failure {
            code: 1,
            message: "controlled guard failure".into(),
        }),
        EventKind::Cancelled,
        EventKind::Completed,
    ] {
        let (mut worker, output) = retained(viewer::MODE_TEXT);
        let source = content();
        let starts: Vec<_> = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        assert!(worker.set_page(fragment(&source, 0, starts[60]), &output));
        let first = paint(&mut worker, &output);
        let info = first.raster.as_ref().unwrap();
        let target = [
            0.0,
            f64::from(worker.shaped.as_ref().unwrap().height()) + 33.25,
        ];
        pan(&mut worker, &output, info, 1, target, false);
        assert!(worker.raster.continuing.is_some());
        worker.viewport_request = Some(41);
        worker.event(41, terminal, &output);
        assert!(
            worker.raster.continuing.is_none(),
            "terminal guard failure cannot keep the camera pending forever"
        );
        worker.paint(&output).unwrap();
        let events = output.take();
        assert!(events.iter().any(|event|matches!(event,ViewerOutput::RasterUnavailable {surface,..} if *surface==info.surface)));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ViewerOutput::Frame(_))),
            "failed target must not acknowledge"
        );
        pan(&mut worker, &output, info, 2, [0.0, 17.25], true);
        let recovered = paint(&mut worker, &output);
        assert_eq!(recovered.raster.as_ref().unwrap().surface, info.surface);
        assert_eq!(recovered.raster.as_ref().unwrap().origin, [0.0, 17.25]);
        assert_eq!(recovered.raster.as_ref().unwrap().pan_sequence, 2);
    }
}

#[test]
fn retained_drag_extension_accepts_displayed_pixels_after_selection_only_repaint() {
    let (mut worker, output) = retained(viewer::MODE_SOURCE);
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    let margin = worker.text_margin();
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Select {
                pixel_id: info.pixel_id,
                origin: info.origin,
                x: margin + 8.0,
                y: 8.0,
                extend: false,
            },
        },
        &output,
    );
    let repainted = paint(&mut worker, &output);
    assert_ne!(repainted.raster.as_ref().unwrap().pixel_id, info.pixel_id);
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Select {
                pixel_id: info.pixel_id,
                origin: info.origin,
                x: margin + 100.0,
                y: 8.0,
                extend: true,
            },
        },
        &output,
    );
    assert!(
        worker.selection.is_some_and(|(a, b)| a != b),
        "selection-only repaint must not invalidate still displayed hit-test coordinates"
    );
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Copy,
        },
        &output,
    );
    assert!(
        output
            .take()
            .into_iter()
            .any(|event| matches!(event,ViewerOutput::CopyText(text) if !text.is_empty()))
    );
}

#[test]
fn retained_selection_during_guard_io_uses_the_still_displayed_page() {
    let (mut worker, output) = retained(viewer::MODE_SOURCE);
    let source = content();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    assert!(worker.set_page(fragment(&source, 0, starts[60]), &output));
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    let target = [
        0.0,
        f64::from(worker.shaped.as_ref().unwrap().height()) + 1.0,
    ];
    pan(&mut worker, &output, info, 1, target, false);
    assert!(worker.raster.continuing.is_some());
    let margin = worker.text_margin();
    let expected = worker.shaped.as_ref().unwrap().hit_test(40.0, 8.0).unwrap();
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Select {
                pixel_id: info.pixel_id,
                origin: info.origin,
                x: margin + 40.0,
                y: 8.0,
                extend: false,
            },
        },
        &output,
    );
    assert_eq!(worker.selection, Some((expected, expected)));
    assert_eq!(worker.origin, [0.0, 0.0]);
    assert!(
        worker.raster.continuing.is_some(),
        "selection does not replace the pending source-anchor mapping"
    );
}

fn bounded_fragment(spare: u32, backwards: bool) -> (Worker, Mailbox, String, usize, usize) {
    let (mut worker, output) = retained(viewer::MODE_TEXT);
    let source: String = (0..120)
        .map(|row| format!("{row:03} {}\n", "x".repeat(280)))
        .collect();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    let (start, end) = if backwards {
        (starts[39], starts[78])
    } else {
        (0, starts[39])
    };
    assert!(end - start <= 12 * 1024);
    worker.length = source.len() as u64;
    assert!(worker.set_page(fragment(&source, start, end), &output));
    let available = crate::viewport::available_height(
        worker.page.as_ref().unwrap(),
        worker.shaped.as_ref().unwrap(),
        100.0,
    );
    worker.geometry.height = (available as u32) - spare;
    assert!(worker.set_page(fragment(&source, start, end), &output));
    (worker, output, source, start, end)
}

#[test]
fn retained_bounded_eighty_pixel_overlap_advances_both_directions_without_jumping_visible_origin() {
    for backwards in [false, true] {
        let (mut worker, output, source, start, end) = bounded_fragment(80, backwards);
        let first = paint(&mut worker, &output);
        let info = first.raster.as_ref().unwrap();
        let span = info.coverage_max[1] - info.coverage_min[1];
        assert!((80.0..81.0).contains(&span));
        assert_eq!(super::super::raster::guard(worker.geometry, true)[1], 256);
        let displayed = if backwards {
            info.coverage_min[1]
        } else {
            info.coverage_max[1]
        };
        let target = [
            0.0,
            displayed + if backwards { -span / 2.0 } else { span / 2.0 },
        ];
        pan(&mut worker, &output, info, 1, target, false);
        let continuation = worker.continuation.as_ref().expect("bounded guard request");
        let anchor = continuation.anchor as usize;
        assert!(
            if backwards {
                anchor < end
            } else {
                anchor > start
            },
            "source anchor must advance with80px overlap (backwards={backwards},anchor={anchor},range={start}..{end})"
        );
        let (next_start, next_end) = if backwards {
            (anchor - (end - start), anchor)
        } else {
            (anchor, anchor + (end - start))
        };
        assert!(next_end - next_start <= 12 * 1024);
        assert!(worker.set_page(fragment(&source, next_start, next_end), &output));
        worker.resume_raster_target(&output);
        let next = paint(&mut worker, &output);
        let prepared = next.raster.as_ref().unwrap();
        assert_eq!(prepared.surface, info.surface);
        assert!(
            prepared.coverage_min[1] <= displayed + 1e-4
                && prepared.coverage_max[1] >= displayed - 1e-4,
            "replacement must cover the displayed frontier"
        );
        assert!(if backwards {
            prepared.coverage_min[1] < displayed
        } else {
            prepared.coverage_max[1] > displayed
        });
        assert!((prepared.origin[1] - target[1]).abs() < 1e-4);
    }
}

#[test]
fn retained_one_pixel_overlap_uses_bounded_fallback_and_accumulates_small_steps_until_source_progress()
 {
    for backwards in [false, true] {
        let (mut worker, output, _source, start, end) = bounded_fragment(1, backwards);
        let first = paint(&mut worker, &output);
        assert!(
            first.raster.is_none(),
            "one pixel cannot preserve and advance a whole source row"
        );
        let mut progressed = false;
        for _ in 0..256 {
            worker.input(
                ViewerInput::Scroll {
                    x: 0.0,
                    y: if backwards { -0.25 } else { 0.25 },
                },
                &output,
            );
            if let Some(continuation) = &worker.continuation {
                assert!(
                    if backwards {
                        continuation.anchor < end as u64
                    } else {
                        continuation.anchor > start as u64
                    },
                    "fallback must never rerequest the identical fragment"
                );
                progressed = true;
                break;
            }
        }
        assert!(
            progressed,
            "fractional wheel input must accumulate to a safe source advance"
        );
    }
}

#[test]
fn retained_tiny_fragment_reversal_discards_unapplied_forward_residual() {
    let (mut worker, output, _, _, _) = bounded_fragment(1, false);
    worker.input(ViewerInput::Scroll { x: 0.0, y: 1.75 }, &output);
    let at = worker.origin[1];
    assert!(worker.raster.fallback_scroll > 0.0);
    worker.input(ViewerInput::Scroll { x: 0.0, y: -0.25 }, &output);
    assert!((worker.origin[1] - (at - 0.25)).abs() < 1e-5);
    assert_eq!(worker.raster.fallback_scroll, 0.0);
}

#[test]
fn retained_wrapped_tab_refill_preserves_logical_context_and_exact_visible_pixels() {
    for backwards in [false, true] {
        let (mut worker, output) = retained(viewer::MODE_TEXT);
        worker.wrap = true;
        worker.geometry.width = 146;
        let source: String = (0..80).map(|row| format!("{row:03} abcdefghijklmno\tPQRSTUVWXYZ abcdefghijklmno\tPQRSTUVWXYZ abcdefghijklmno\tPQRSTUVWXYZ\n")).collect();
        let starts: Vec<_> = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        let (start, end) = if backwards {
            (starts[20], starts[40])
        } else {
            (0, starts[20])
        };
        worker.length = source.len() as u64;
        assert!(worker.set_page(fragment(&source, start, end), &output));
        let first = paint(&mut worker, &output);
        let info = first.raster.as_ref().unwrap();
        let maximum = crate::viewport::available_height(
            worker.page.as_ref().unwrap(),
            worker.shaped.as_ref().unwrap(),
            worker.geometry.height as f32,
        ) - worker.geometry.height as f32;
        let target = [
            0.0,
            if backwards {
                32.25
            } else {
                f64::from(maximum) - 32.25
            },
        ];
        let margin = worker.text_margin();
        let expected = worker
            .renderer
            .rasterize(
                worker.shaped.as_ref().unwrap(),
                Viewport {
                    width: worker.geometry.width,
                    height: worker.geometry.height,
                    origin: [-margin, target[1] as f32],
                    background: worker.appearance.background,
                    selection_color: worker.appearance.selection,
                },
                None,
            )
            .unwrap();
        pan(&mut worker, &output, info, 1, target, false);
        let scroll = worker.position_metrics().unwrap();
        let anchor = worker.continuation.as_ref().expect("guard refill").anchor as usize;
        assert!(
            anchor > 0 && source.as_bytes()[anchor - 1] == b'\n',
            "refill must preserve logical-line tab/wrap context, anchor={anchor}, backwards={backwards}"
        );
        let (next_start, next_end) = if backwards {
            (anchor - (end - start), anchor)
        } else {
            (anchor, anchor + (end - start))
        };
        assert!(next_end - next_start <= 12 * 1024);
        assert!(worker.set_page(fragment(&source, next_start, next_end), &output));
        worker.resume_raster_target(&output);
        let next = paint(&mut worker, &output);
        assert_eq!(next.raster.as_ref().unwrap().surface, info.surface);
        assert!((next.position - scroll.0).abs() < 1e-6);
        assert!((next.vertical_extent - scroll.1).abs() < 1e-6);
        assert_pixels_equal(
            &visible(&next),
            &expected.rgba,
            "wrapped tab refill must preserve shown pixels",
        );
    }
}

#[test]
fn retained_wrapped_partial_first_line_replaces_context_when_backward_refill_adds_prefix() {
    let (mut worker, output) = retained(viewer::MODE_TEXT);
    worker.wrap = true;
    worker.geometry.width = 146;
    let source: String = (0..80).map(|row| format!("{row:03} abcdefghijklmno\tPQRSTUVWXYZ abcdefghijklmno\tPQRSTUVWXYZ abcdefghijklmno\tPQRSTUVWXYZ\n")).collect();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    let (start, end) = (starts[20] + 10, starts[40]);
    worker.length = source.len() as u64;
    assert!(worker.set_page(fragment(&source, start, end), &output));
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    pan(&mut worker, &output, info, 1, [0.0, 32.25], false);
    let anchor = worker.continuation.as_ref().expect("backward guard").anchor as usize;
    assert!(anchor > start && anchor < end);
    // A preceding complete source prefix now changes the first old logical line.
    assert!(worker.set_page(fragment(&source, starts[5], anchor), &output));
    worker.resume_raster_target(&output);
    worker.paint(&output).unwrap();
    let transition = output
        .take()
        .into_iter()
        .find_map(|message| match message {
            ViewerOutput::RasterRebase {
                surface,
                transition,
            } if surface == info.surface => Some(transition),
            ViewerOutput::Frame(_) => {
                panic!("partial-prefix pixels require a displayed-camera handoff")
            }
            _ => None,
        })
        .expect("partial-prefix handoff");
    worker.input(
        ViewerInput::RasterRebase {
            transition,
            origin: info.origin,
        },
        &output,
    );
    let next = paint(&mut worker, &output);
    assert_eq!(next.raster_rebase, Some(transition));
    assert_ne!(
        next.raster.as_ref().unwrap().surface,
        info.surface,
        "a newly available prefix cannot claim the old wrapped pixel context"
    );
}

fn pending_rebase() -> (
    Worker,
    Mailbox,
    String,
    usize,
    ViewerRasterInfo,
    f64,
    f32,
    u64,
) {
    let (mut worker, output) = retained(viewer::MODE_TEXT);
    worker.wrap = true;
    worker.geometry.width = 146;
    let prefix: String = (0..20).map(|row| format!("short {row:03}\n")).collect();
    let start = prefix.len();
    let source = prefix + &"abcdefghijklmno\tPQRSTUVWXYZ".repeat(800) + "\nlast\n";
    worker.length = source.len() as u64;
    assert!(worker.set_page(fragment(&source, 0, 8192), &output));
    let maximum = crate::viewport::available_height(
        worker.page.as_ref().unwrap(),
        worker.shaped.as_ref().unwrap(),
        worker.geometry.height as f32,
    ) - worker.geometry.height as f32;
    worker.origin[1] = maximum - 148.0;
    worker.selection = Some((0, 5));
    worker.reset_raster_context();
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap().clone();
    let displayed = info.origin[1];
    pan(
        &mut worker,
        &output,
        &info,
        1,
        [0.0, displayed + 128.0],
        false,
    );
    assert_eq!(worker.continuation.as_ref().unwrap().anchor, start as u64);
    assert!(worker.set_page(fragment(&source, start, start + 8192), &output));
    let speculative_local = worker.origin[1];
    worker.paint(&output).unwrap();
    let messages = output.take();
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, ViewerOutput::Frame(_))),
        "automatic fallback must not publish the speculative prefetch camera"
    );
    let transition = messages
        .into_iter()
        .find_map(|message| match message {
            ViewerOutput::RasterRebase {
                surface,
                transition,
            } => {
                assert_eq!(surface, info.surface);
                Some(transition)
            }
            _ => None,
        })
        .expect("displayed-camera handoff request");
    (
        worker,
        output,
        source,
        start,
        info,
        displayed,
        speculative_local,
        transition,
    )
}

#[test]
fn retained_rebase_holds_prefetch_pixels_until_displayed_camera_reply_and_keeps_ack_sticky() {
    let (mut worker, output, _, _, info, displayed, speculative, transition) = pending_rebase();
    worker.input(
        ViewerInput::RasterRebase {
            transition: transition + 1,
            origin: [0.0, displayed],
        },
        &output,
    );
    worker.input(
        ViewerInput::RasterRebase {
            transition,
            origin: [0.0, f64::NAN],
        },
        &output,
    );
    pan(
        &mut worker,
        &output,
        &info,
        2,
        [0.0, displayed + 200.0],
        false,
    );
    worker.paint(&output).unwrap();
    assert!(
        !output
            .take()
            .iter()
            .any(|message| matches!(message, ViewerOutput::Frame(_)))
    );
    worker.input(
        ViewerInput::RasterRebase {
            transition,
            origin: [0.0, displayed],
        },
        &output,
    );
    for _ in 0..100 {
        output.put(ViewerOutput::WebShortcut(
            midnight_core::viewer_web::HostShortcut::Find,
        ));
    }
    let corrected = paint(&mut worker, &output);
    assert_eq!(corrected.raster_rebase, Some(transition));
    assert!(corrected.raster.is_none());
    assert!((worker.origin[1] - (speculative - 128.0)).abs() < 0.001);
    let origin = worker.origin;
    worker.input(
        ViewerInput::RasterRebase {
            transition,
            origin: [0.0, displayed + 500.0],
        },
        &output,
    );
    assert_eq!(
        worker.origin, origin,
        "duplicate reply must not move the corrected camera"
    );
    assert_eq!(
        paint(&mut worker, &output).raster_rebase,
        Some(transition),
        "coalesced metadata frame must retain the acknowledgement"
    );
    worker.input(ViewerInput::Scroll { x: 0.0, y: 1.0 }, &output);
    assert_eq!(paint(&mut worker, &output).raster_rebase, None);
}

#[test]
fn retained_rebase_explicit_navigation_cancels_the_pending_displayed_camera_authority() {
    let (mut worker, output, _, _, _, displayed, _, transition) = pending_rebase();
    worker.input(ViewerInput::Seek(0.0), &output);
    let origin = worker.origin;
    worker.input(
        ViewerInput::RasterRebase {
            transition,
            origin: [0.0, displayed],
        },
        &output,
    );
    assert_eq!(worker.origin, origin);
    assert_eq!(paint(&mut worker, &output).raster_rebase, None);
}

#[test]
fn retained_rebase_waits_for_bounded_correction_page_terminal_before_acknowledging() {
    let (mut worker, output, source, start, _, displayed, speculative, transition) =
        pending_rebase();
    // Restore a source camera in the previous bounded prefix, requiring one backward read.
    let requested = displayed + 128.0 - f64::from(speculative) - 20.0;
    worker.input(
        ViewerInput::RasterRebase {
            transition,
            origin: [0.0, requested],
        },
        &output,
    );
    assert!(
        worker
            .continuation
            .as_ref()
            .is_some_and(|next| next.backwards && next.anchor < (start + 8192) as u64)
    );
    worker.viewport_request = Some(91);
    worker.paint(&output).unwrap();
    assert!(
        !output
            .take()
            .iter()
            .any(|message| matches!(message, ViewerOutput::Frame(_)))
    );
    assert!(worker.set_page(fragment(&source, 0, 8192), &output));
    worker.paint(&output).unwrap();
    assert!(
        !output
            .take()
            .iter()
            .any(|message| matches!(message, ViewerOutput::Frame(_)))
    );
    worker.event(91, EventKind::Completed, &output);
    assert_eq!(paint(&mut worker, &output).raster_rebase, Some(transition));
    let _ = speculative;
}

#[test]
fn retained_rebase_failed_or_cancelled_correction_clears_wait_without_success_ack() {
    for event in [
        EventKind::Cancelled,
        EventKind::Completed,
        EventKind::Failed(messages::Failure {
            code: 1,
            message: "correction rejected".into(),
        }),
    ] {
        let (mut worker, output, _, _, info, displayed, speculative, transition) = pending_rebase();
        worker.input(
            ViewerInput::RasterRebase {
                transition,
                origin: [0.0, displayed + 128.0 - f64::from(speculative) - 20.0],
            },
            &output,
        );
        assert!(worker.continuation.is_some());
        worker.viewport_request = Some(92);
        for _ in 0..100 {
            output.put(ViewerOutput::WebShortcut(
                midnight_core::viewer_web::HostShortcut::Find,
            ));
        }
        worker.event(92, event, &output);
        worker.paint(&output).unwrap();
        let messages = output.take();
        let error=messages.iter().position(|message|matches!(message,ViewerOutput::RasterUnavailable{surface,..} if *surface==info.surface)).expect("handoff failure survives pressure");
        let frame=messages.iter().position(|message|matches!(message,ViewerOutput::Frame(frame) if frame.raster_rebase.is_none())).expect("usable unacknowledged frame survives pressure");
        assert!(
            error < frame,
            "failure control must precede the replacement frame"
        );
        if let (ViewerOutput::RasterUnavailable { message, .. }, ViewerOutput::Frame(frame)) =
            (&messages[error], &messages[frame])
        {
            assert!(
                frame.status.contains(message),
                "replacement frame must retain its preceding visible error"
            );
        }
        let origin = worker.origin;
        worker.input(
            ViewerInput::RasterRebase {
                transition,
                origin: [0.0, displayed],
            },
            &output,
        );
        assert_eq!(worker.origin, origin);
    }
}

#[test]
fn retained_rebase_missing_anchor_cannot_acknowledge_an_uncovered_camera() {
    let (mut worker, output, _, _, info, displayed, speculative, transition) = pending_rebase();
    worker.page.as_mut().unwrap().anchors.clear();
    worker.input(
        ViewerInput::RasterRebase {
            transition,
            origin: [0.0, displayed + 128.0 - f64::from(speculative) - 20.0],
        },
        &output,
    );
    assert!(output.take().iter().any(|message|matches!(message,ViewerOutput::RasterUnavailable{surface,..} if *surface==info.surface)));
    assert_eq!(paint(&mut worker, &output).raster_rebase, None);
}

#[test]
fn retained_rebase_preserves_selected_text_but_rejected_new_selection_blocks_old_copy() {
    for reject_selection in [false, true] {
        let (mut worker, output, _, _, info, displayed, _, transition) = pending_rebase();
        if reject_selection {
            worker.input(
                ViewerInput::Raster {
                    surface: info.surface,
                    input: ViewerRasterInput::Select {
                        pixel_id: info.pixel_id,
                        origin: info.origin,
                        x: 10.0,
                        y: 10.0,
                        extend: false,
                    },
                },
                &output,
            );
        }
        worker.input(
            ViewerInput::RasterRebase {
                transition,
                origin: [0.0, displayed],
            },
            &output,
        );
        let frame = paint(&mut worker, &output);
        worker.input(
            ViewerInput::Copy {
                frame: frame.revision,
            },
            &output,
        );
        let copied = output.take().into_iter().find_map(|event| {
            if let ViewerOutput::CopyText(text) = event {
                Some(text)
            } else {
                None
            }
        });
        assert_eq!(copied, (!reject_selection).then(|| "short".to_owned()));
    }
}

#[test]
fn retained_rebase_resize_cancels_pending_correction_and_rejects_late_page_or_reply() {
    let (mut worker, output, _, start, _, displayed, speculative, transition) = pending_rebase();
    worker.input(
        ViewerInput::RasterRebase {
            transition,
            origin: [0.0, displayed + 128.0 - f64::from(speculative) - 20.0],
        },
        &output,
    );
    assert!(worker.continuation.is_some());
    worker.viewport_request = Some(93);
    worker.input(
        ViewerInput::Resize(ViewerGeometry {
            width: 160,
            height: 100,
            scale: 1.0,
        }),
        &output,
    );
    assert!(worker.viewport_request.is_none());
    assert!(worker.continuation.is_none());
    worker.event(
        93,
        EventKind::TextPage(decoded_page("stale correction")),
        &output,
    );
    worker.event(93, EventKind::Completed, &output);
    worker.input(
        ViewerInput::RasterRebase {
            transition,
            origin: [0.0, displayed],
        },
        &output,
    );
    assert_eq!(worker.page.as_ref().unwrap().start, start as u64);
    assert_eq!(paint(&mut worker, &output).raster_rebase, None);
}

#[test]
fn retained_rebase_failed_layout_keeps_correction_authority_until_terminal_failure() {
    let (mut worker, output, _, _, info, displayed, speculative, transition) = pending_rebase();
    worker.input(
        ViewerInput::RasterRebase {
            transition,
            origin: [0.0, displayed + 128.0 - f64::from(speculative) - 20.0],
        },
        &output,
    );
    worker.viewport_request = Some(94);
    let mut invalid = worker.page.as_ref().unwrap().clone();
    invalid.text = "x".repeat(midnight_viewer_render::MAX_TEXT_BYTES + 1);
    assert!(!worker.set_page(invalid, &output));
    worker.paint(&output).unwrap();
    assert!(
        !output
            .take()
            .iter()
            .any(|message| matches!(message, ViewerOutput::Frame(_)))
    );
    worker.event(94, EventKind::Completed, &output);
    assert!(output.take().iter().any(|message|matches!(message,ViewerOutput::RasterUnavailable{surface,..} if *surface==info.surface)));
    assert_eq!(paint(&mut worker, &output).raster_rebase, None);
}

#[test]
fn retained_scrollbar_during_uncompleted_rebase_cannot_acknowledge_the_speculative_page() {
    let (mut worker, output, source, _, _, displayed, speculative, transition) = pending_rebase();
    worker.input(
        ViewerInput::RasterRebase {
            transition,
            origin: [0.0, displayed + 128.0 - f64::from(speculative) - 20.0],
        },
        &output,
    );
    worker.viewport_request = Some(91);
    assert!(worker.set_page(fragment(&source, 0, 8192), &output));
    let page = worker.page.as_ref().unwrap();
    let position = navigation::scroll_metrics(
        page.start,
        page.end,
        worker.length,
        worker.shaped.as_ref().unwrap().height(),
        worker.geometry.height as f32,
        worker.origin[1],
    )
    .0;
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
    assert_eq!(
        worker.positions.acknowledged[0], 0,
        "uncompleted read was mistaken for a committed page"
    );
    worker.event(91, EventKind::Completed, &output);
    assert_eq!(
        worker.positions.acknowledged[0], 0,
        "obsolete completion must not acknowledge the new seek"
    );
}

#[path = "worker_raster_scroll_tests.rs"]
mod scroll_mapping;
