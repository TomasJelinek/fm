use super::*;
use std::sync::Arc;

fn frame(revision: u64) -> ViewerOutput {
    let ViewerOutput::Frame(mut frame) = text_presentation_tests::frame(revision, 0, 0.2) else {
        unreachable!()
    };
    frame.mode = 2;
    frame.modes = vec![1, 2];
    frame.width = 600;
    frame.height = 600;
    frame.pixels = vec![255; 600 * 600 * 4].into();
    frame.raster = Some(ViewerRasterInfo {
        surface: ViewerSurfaceId(90),
        pixel_id: 1,
        origin: [100.0, 200.0],
        raster_origin: [0.0, 0.0],
        viewport: [400, 240],
        coverage_min: [0.0; 2],
        coverage_max: [200.0, 360.0],
        limit_min: [Some(0.0); 2],
        limit_max: [None; 2],
        pan_sequence: 0,
        scroll_per_pixel: [0.001, 0.0001],
        settled: true,
        gutter: Some(ViewerRasterGutter {
            width: 20,
            height: 600,
            pixels: Arc::from(vec![128; 20 * 600 * 4]),
        }),
    });
    ViewerOutput::Frame(frame)
}

#[test]
fn retained_text_pixels_move_before_worker_ack_and_metadata_reuses_uploaded_images() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    let original = frame(10);
    present_outputs(&open, &service, &|_| {}, vec![original.clone()]);
    assert!(open.window.get_retained_raster());
    assert!(!open.window.get_image_mode());
    assert!(open.window.get_source_mode());
    let gutter_width = open.window.get_raster_gutter_width();
    let initial = [
        open.window.get_raster_offset_x(),
        open.window.get_raster_offset_y(),
    ];
    let uploads = open.window.get_raster_upload_count();
    open.window.invoke_scroll(3.5, 7.25);
    assert!(open.window.get_raster_offset_x() < initial[0]);
    assert!(open.window.get_raster_offset_y() < initial[1]);
    assert_eq!(open.window.get_raster_gutter_width(), gutter_width);
    assert_eq!(open.window.get_raster_upload_count(), uploads);
    let displayed = open.retained_raster.borrow().camera.origin;
    let ViewerOutput::Frame(mut ack) = original else {
        unreachable!()
    };
    ack.revision = 11;
    present_outputs(&open, &service, &|_| {}, vec![ViewerOutput::Frame(ack)]);
    assert_eq!(open.retained_raster.borrow().camera.origin, displayed);
    assert_eq!(open.window.get_raster_upload_count(), uploads);
    manager.close_all();
}

#[test]
fn retained_selection_uses_displayed_camera_and_effective_viewport_not_overscan_dimensions() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let received = Rc::new(RefCell::new(Vec::new()));
    let sink = received.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        sink.borrow_mut().push(input);
        true
    });
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    open.window.invoke_scroll(10.0, 20.0);
    let origin = open.retained_raster.borrow().camera.origin;
    let half_x = open.window.get_raster_viewport_width() / 2.0;
    let half_y = open.window.get_raster_viewport_height() / 2.0;
    open.window.invoke_select_point(half_x, half_y, false);
    assert!(received.borrow().iter().any(|input| matches!(input,
        ViewerInput::Raster { surface: ViewerSurfaceId(90), input: ViewerRasterInput::Select {
            pixel_id: 1, origin: selected, x: 200.0, y: 120.0, extend: false,
        }} if *selected == origin)));
    assert!(dispatch_key(&open, "viewer.select-page"));
    assert!(received.borrow().iter().any(|input| matches!(input,
        ViewerInput::Raster { input: ViewerRasterInput::SelectPage { pixel_id: 1, origin: selected }, .. }
            if *selected == origin)));
    manager.close_all();
}

#[test]
fn switching_content_keeps_old_pixels_but_prevents_motion_until_new_surface_arrives() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    assert!(open.enqueue(ViewerInput::Search {
        query: "needle".into(),
        backwards: false,
        case_sensitive: false
    }));
    assert!(open.window.get_retained_raster());
    assert!(!raster_presentation::push(&open, 0.0, 60.0, false));
    let ViewerOutput::Frame(mut new) = frame(11) else {
        unreachable!()
    };
    new.raster.as_mut().unwrap().surface = ViewerSurfaceId(91);
    present_outputs(&open, &service, &|_| {}, vec![ViewerOutput::Frame(new)]);
    assert!(raster_presentation::push(&open, 0.0, 60.0, false));
    manager.close_all();
}

#[test]
fn precise_deltas_accumulate_while_the_worker_has_not_yet_refilled_the_visible_frontier() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    let ViewerOutput::Frame(mut short) = frame(10) else {
        unreachable!()
    };
    short.raster.as_mut().unwrap().coverage_max[1] = 205.0;
    present_outputs(&open, &service, &|_| {}, vec![ViewerOutput::Frame(short)]);
    let scale_y = 240.0 / f64::from(open.window.get_raster_viewport_height());
    open.window.invoke_scroll(0.0, 100.0);
    assert!((open.retained_raster.borrow().camera.origin[1] - 205.0).abs() < 0.00001);
    open.window.invoke_scroll(0.0, 100.0);
    let ViewerOutput::Frame(mut more) = frame(11) else {
        unreachable!()
    };
    more.raster.as_mut().unwrap().pixel_id = 2;
    present_outputs(&open, &service, &|_| {}, vec![ViewerOutput::Frame(more)]);
    raster_presentation::step(&open);
    let expected = 200.0 + 200.0 * scale_y;
    assert!(
        (open.retained_raster.borrow().camera.origin[1] - expected).abs() < 0.001,
        "a precise scroll delta was lost at the frontier"
    );
    manager.close_all();
}

#[test]
fn invalid_retained_pixel_dimensions_leave_the_current_picture_intact() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    let before = open.retained_raster.borrow().camera.origin;
    let uploads = open.window.get_raster_upload_count();
    let ViewerOutput::Frame(mut bad) = frame(11) else {
        unreachable!()
    };
    bad.width = u32::MAX;
    bad.height = u32::MAX;
    present_outputs(&open, &service, &|_| {}, vec![ViewerOutput::Frame(bad)]);
    assert_eq!(open.retained_raster.borrow().camera.origin, before);
    assert_eq!(open.window.get_raster_upload_count(), uploads);
    manager.close_all();
}

#[test]
fn a_guard_failure_keeps_visible_pixels_and_stops_motion_but_an_old_context_error_is_ignored() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    raster_presentation::push(&open, 0.0, 60.0, false);
    image_presentation_tests::advance_time(Duration::from_millis(16));
    raster_presentation::step(&open);
    let origin = open.retained_raster.borrow().camera.origin;
    let uploads = open.window.get_raster_upload_count();
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::RasterUnavailable {
            surface: ViewerSurfaceId(90),
            message: "Source became unavailable".into(),
        }],
    );
    image_presentation_tests::advance_time(Duration::from_millis(16));
    raster_presentation::step(&open);
    assert_eq!(open.retained_raster.borrow().camera.origin, origin);
    assert_eq!(open.window.get_raster_upload_count(), uploads);
    assert!(open.window.get_retained_raster());
    assert!(!open.retained_raster.borrow().camera.active());
    let ViewerOutput::Frame(mut next) = frame(11) else {
        unreachable!()
    };
    next.raster.as_mut().unwrap().surface = ViewerSurfaceId(91);
    present_outputs(&open, &service, &|_| {}, vec![ViewerOutput::Frame(next)]);
    assert!(raster_presentation::push(&open, 0.0, -30.0, false));
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::RasterUnavailable {
            surface: ViewerSurfaceId(90),
            message: "Obsolete failure".into(),
        }],
    );
    assert!(open.retained_raster.borrow().camera.active());
    assert_ne!(open.window.get_status().as_str(), "Obsolete failure");
    manager.close_all();
}

fn scrollbar_capture(open: &OpenWindow) -> Rc<RefCell<Vec<ViewerInput>>> {
    open.timer.stop();
    open.image_redraw.stop();
    let received = Rc::new(RefCell::new(Vec::new()));
    let sink = received.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        sink.borrow_mut().push(input);
        true
    });
    received
}

#[test]
fn retained_scrollbar_partial_page_seek_and_pan_present_immediately_without_upload_or_position_read()
 {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let received = scrollbar_capture(&open);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    let uploads = open.window.get_raster_upload_count();
    open.window.invoke_seek(0.2075);
    assert!((open.retained_raster.borrow().camera.origin[1] - 275.0).abs() < 0.001);
    open.window.invoke_pan(0.05);
    assert!((open.retained_raster.borrow().camera.origin[0] - 150.0).abs() < 0.001);
    assert!(!open.text_position.borrow().has_pending());
    assert_eq!(open.window.get_raster_upload_count(), uploads);
    assert!(received.borrow().iter().all(|input| matches!(
        input,
        ViewerInput::Raster {
            input: ViewerRasterInput::PanTo { settled: true, .. },
            ..
        }
    )));
    assert!(received.borrow().len() >= 2);
    assert!((open.window.get_displayed_position() - 0.2075).abs() < 0.00001);
    manager.close_all();
}

#[test]
fn retained_scrollbar_uncovered_target_keeps_the_exact_position_fallback_without_clamping() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let received = scrollbar_capture(&open);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    open.window.invoke_seek(0.9);
    assert_eq!(open.retained_raster.borrow().camera.origin, [100.0, 200.0]);
    assert!(received.borrow().iter().any(|input| matches!(input,
        ViewerInput::Position { horizontal: false, position, .. } if (*position - 0.9).abs() < 0.000001)));
    assert!(open.text_position.borrow().has_pending());
    assert!(
        received
            .borrow()
            .iter()
            .all(|input| !matches!(input, ViewerInput::Raster { .. }))
    );
    manager.close_all();
}

#[test]
fn retained_scrollbar_pending_position_or_rebase_never_reuses_the_old_mapping() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let received = scrollbar_capture(&open);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    open.text_position.borrow_mut().request(false, 0.8);
    open.window.invoke_seek(0.205);
    assert_eq!(open.retained_raster.borrow().camera.origin, [100.0, 200.0]);
    assert!(
        received
            .borrow()
            .iter()
            .any(|input| matches!(input, ViewerInput::Position { .. }))
    );
    let ViewerOutput::Frame(mut next) = frame(11) else {
        unreachable!()
    };
    next.generation = 8;
    next.raster.as_mut().unwrap().surface = ViewerSurfaceId(91);
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![
            ViewerOutput::Frame(next),
            ViewerOutput::RasterRebase {
                surface: ViewerSurfaceId(91),
                transition: 1,
            },
        ],
    );
    assert!(raster_rebase::waiting(&open));
    received.borrow_mut().clear();
    open.window.invoke_seek(0.205);
    assert!(
        !raster_rebase::waiting(&open),
        "absolute fallback cancels the obsolete handoff"
    );
    assert!(
        received
            .borrow()
            .iter()
            .any(|input| matches!(input, ViewerInput::Position { generation: 8, .. }))
    );
    assert!(
        received
            .borrow()
            .iter()
            .all(|input| !matches!(input, ViewerInput::Raster { .. }))
    );
    manager.close_all();
}

#[test]
fn retained_scrollbar_latest_reversal_survives_backpressure_and_stale_ack_without_upload() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let allowed = Rc::new(Cell::new(false));
    let accepted = Rc::new(RefCell::new(Vec::new()));
    let sink = accepted.clone();
    let accept = allowed.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        if !accept.get() {
            return false;
        }
        sink.borrow_mut().push(input);
        true
    });
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    let uploads = open.window.get_raster_upload_count();
    open.window.invoke_seek(0.205);
    assert!((open.retained_raster.borrow().camera.origin[1] - 250.0).abs() < 0.001);
    open.window.invoke_seek(0.195);
    assert!((open.retained_raster.borrow().camera.origin[1] - 150.0).abs() < 0.001);
    allowed.set(true);
    raster_motion::step(&open);
    let last = accepted
        .borrow()
        .iter()
        .find_map(|input| match input {
            ViewerInput::Raster {
                input:
                    ViewerRasterInput::PanTo {
                        origin,
                        settled: true,
                        ..
                    },
                ..
            } => Some(*origin),
            _ => None,
        })
        .expect("latest absolute camera retry");
    assert!((last[1] - 150.0).abs() < 0.001);
    let ViewerOutput::Frame(mut stale) = frame(11) else {
        unreachable!()
    };
    stale.position = 0.205;
    stale.raster.as_mut().unwrap().origin[1] = 250.0;
    stale.raster.as_mut().unwrap().pan_sequence = 1;
    present_outputs(&open, &service, &|_| {}, vec![ViewerOutput::Frame(stale)]);
    assert!((open.retained_raster.borrow().camera.origin[1] - 150.0).abs() < 0.001);
    assert_eq!(open.window.get_raster_upload_count(), uploads);
    assert!(!open.text_position.borrow().has_pending());
    manager.close_all();
}

#[test]
fn retained_scrollbar_real_pointer_release_sends_final_settle_without_an_extra_move() {
    use slint::platform::{PointerEventButton, WindowEvent};
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let received = scrollbar_capture(&open);
    present_outputs(&open, &service, &|_| {}, vec![frame(10)]);
    image_presentation_tests::bitmap();
    let bar = &open.window;
    let height = bar.get_vertical_bar_height();
    let thumb = (height * bar.get_vertical_extent()).max(28.0).min(height);
    let start = slint::LogicalPosition::new(
        bar.get_vertical_bar_left() + bar.get_vertical_bar_width() / 2.0,
        bar.get_vertical_bar_top() + bar.get_position() * (height - thumb) + thumb / 2.0,
    );
    let moved = slint::LogicalPosition::new(start.x, start.y + (height - thumb) * 0.005);
    bar.window()
        .dispatch_event(WindowEvent::PointerMoved { position: start });
    bar.window().dispatch_event(WindowEvent::PointerPressed {
        position: start,
        button: PointerEventButton::Left,
    });
    bar.window()
        .dispatch_event(WindowEvent::PointerMoved { position: moved });
    let before = received
        .borrow()
        .iter()
        .filter_map(|input| match input {
            ViewerInput::Raster {
                input:
                    ViewerRasterInput::PanTo {
                        origin, settled, ..
                    },
                ..
            } => Some((*origin, *settled)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        before.iter().any(|(_, settled)| !settled),
        "drag must present without exact-phase work at each move"
    );
    assert!((open.retained_raster.borrow().camera.origin[1] - 250.0).abs() < 0.01);
    bar.window().dispatch_event(WindowEvent::PointerReleased {
        position: moved,
        button: PointerEventButton::Left,
    });
    let after = received
        .borrow()
        .iter()
        .filter_map(|input| match input {
            ViewerInput::Raster {
                input:
                    ViewerRasterInput::PanTo {
                        origin, settled, ..
                    },
                ..
            } => Some((*origin, *settled)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(after.len() > before.len());
    assert!(after.last().unwrap().1);
    assert_eq!(after.last().unwrap().0, before.last().unwrap().0);
    received.borrow_mut().clear();
    open.window.invoke_position_drag_began(false);
    open.window.invoke_drag_seek(false, 0.206);
    raster_motion::cancel(&open);
    raster_motion::step(&open);
    assert!(received.borrow().iter().any(|input| matches!(
        input,
        ViewerInput::Raster {
            input: ViewerRasterInput::PanTo { settled: true, .. },
            ..
        }
    )));
    let count = received.borrow().len();
    open.window.invoke_position_drag_ended(false);
    assert_eq!(
        received.borrow().len(),
        count,
        "late end after focus/pointer cancel is inert"
    );
    manager.close_all();
}

#[test]
fn retained_scrollbar_moves_real_body_and_gutter_pixels_before_any_worker_reply() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    scrollbar_capture(&open);
    let ViewerOutput::Frame(mut colored) = frame(10) else {
        unreachable!()
    };
    let mut body = vec![0_u8; 600 * 600 * 4];
    for y in 0..600 {
        for x in 0..600 {
            let color = if (230..250).contains(&x) && (300..320).contains(&y) {
                [253, 27, 15, 255]
            } else {
                [15, 60, 10, 255]
            };
            body[(y * 600 + x) * 4..(y * 600 + x + 1) * 4].copy_from_slice(&color);
        }
    }
    colored.pixels = body.into();
    let mut gutter = vec![0_u8; 20 * 600 * 4];
    for y in 0..600 {
        for x in 0..20 {
            let color = if (3..8).contains(&x) && (300..320).contains(&y) {
                [17, 241, 253, 255]
            } else {
                [10, 15, 90, 255]
            };
            gutter[(y * 20 + x) * 4..(y * 20 + x + 1) * 4].copy_from_slice(&color);
        }
    }
    colored
        .raster
        .as_mut()
        .unwrap()
        .gutter
        .as_mut()
        .unwrap()
        .pixels = gutter.into();
    present_outputs(&open, &service, &|_| {}, vec![ViewerOutput::Frame(colored)]);
    let bounds = |color: [u8; 3]| {
        let (size, pixels) = image_presentation_tests::bitmap();
        let mut result = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
        for (at, pixel) in pixels.iter().enumerate() {
            if [pixel.r, pixel.g, pixel.b] == color {
                let x = (at % size.width as usize) as i32;
                let y = (at / size.width as usize) as i32;
                result = [
                    result[0].min(x),
                    result[1].min(y),
                    result[2].max(x),
                    result[3].max(y),
                ];
            }
        }
        assert!(
            result[0] < result[2] && result[1] < result[3],
            "marker {color:?} must be painted"
        );
        result
    };
    let body_before = bounds([253, 27, 15]);
    let gutter_before = bounds([17, 241, 253]);
    let uploads = open.window.get_raster_upload_count();
    open.window.invoke_seek(0.202);
    open.window.invoke_pan(0.01);
    let body_after = bounds([253, 27, 15]);
    let gutter_after = bounds([17, 241, 253]);
    let scale = open.window.window().scale_factor();
    let dx = 10.0 * open.window.get_raster_viewport_width() / 400.0 * scale;
    let dy = 20.0 * open.window.get_raster_viewport_height() / 240.0 * scale;
    assert!(((body_before[0] - body_after[0]) as f32 - dx).abs() < 1.1);
    assert!(((body_before[1] - body_after[1]) as f32 - dy).abs() < 1.1);
    assert_eq!(
        gutter_before[0], gutter_after[0],
        "gutter remains horizontally pinned"
    );
    assert!(((gutter_before[1] - gutter_after[1]) as f32 - dy).abs() < 1.1);
    assert_eq!(open.window.get_raster_upload_count(), uploads);
    manager.close_all();
}
