use super::*;
use slint::platform::{PointerEventButton, WindowEvent};
use std::sync::Arc;

pub(super) fn frame(revision: u64, sequence: u64, position: f64) -> ViewerOutput {
    ViewerOutput::Frame(Box::new(ViewerFrame {
        generation: 7,
        raster: None,
        raster_rebase: None,
        position_sequence: [sequence, 0],
        revision,
        mode: 1,
        modes: vec![1],
        wrap_enabled: false,
        wrap_available: true,
        image: None,
        width: 2,
        height: 2,
        pixels: vec![255; 16].into(),
        tiles: None,
        accessible_text: Arc::from("Captured text"),
        position,
        vertical_extent: 0.1,
        horizontal_position: 0.0,
        horizontal_extent: 1.0,
        caption: "Synthetic text".into(),
        status: "Ready".into(),
    }))
}

#[test]
fn text_scrollbar_holds_latest_thumb_and_keeps_old_content_until_exact_ack_after_release() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    let bar = &open.window;
    let height = bar.get_vertical_bar_height();
    assert!(height > 100.0);
    let thumb = (height * bar.get_vertical_extent()).max(28.0).min(height);
    let at = |position| {
        slint::LogicalPosition::new(
            bar.get_vertical_bar_left() + bar.get_vertical_bar_width() / 2.0,
            bar.get_vertical_bar_top() + position * (height - thumb) + thumb / 2.0,
        )
    };
    bar.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at(0.2) });
    bar.window().dispatch_event(WindowEvent::PointerPressed {
        position: at(0.2),
        button: PointerEventButton::Left,
    });
    bar.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at(0.8) });
    assert!((open.window.get_position() - 0.8).abs() < 0.0001);
    assert!((open.window.get_displayed_position() - 0.2).abs() < 0.0001);
    present_outputs(&open, &service, &|_| {}, vec![frame(11, 0, 0.3)]);
    assert!((open.window.get_position() - 0.8).abs() < 0.0001);
    assert!((open.window.get_displayed_position() - 0.2).abs() < 0.0001);
    bar.window().dispatch_event(WindowEvent::PointerReleased {
        position: at(0.8),
        button: PointerEventButton::Left,
    });
    present_outputs(&open, &service, &|_| {}, vec![frame(12, 0, 0.4)]);
    assert!((bar.get_vertical_thumb_position() - 0.8).abs() < 0.0001);
    assert!((bar.get_displayed_position() - 0.2).abs() < 0.0001);
    present_outputs(&open, &service, &|_| {}, vec![frame(13, 1, 0.801)]);
    assert!((open.window.get_position() - 0.801).abs() < 0.0001);
    assert!((open.window.get_displayed_position() - 0.801).abs() < 0.0001);
    manager.close_all();
}

#[test]
fn same_mode_selection_keeps_current_position_context_usable() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    open.window.invoke_change_mode(0);
    open.window.invoke_seek(0.7);
    assert!((open.window.get_position() - 0.7).abs() < 0.0001);
    present_outputs(&open, &service, &|_| {}, vec![frame(11, 1, 0.701)]);
    assert!((open.window.get_displayed_position() - 0.701).abs() < 0.0001);
    manager.close_all();
}

#[test]
fn newer_completed_content_is_presented_during_drag_without_clearing_latest_thumb() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    open.window.invoke_seek(0.5);
    open.window.invoke_seek(0.8);
    assert_eq!(submitted.borrow().len(), 2);
    present_outputs(&open, &service, &|_| {}, vec![frame(11, 1, 0.501)]);
    assert!((open.window.get_position() - 0.8).abs() < 0.0001);
    assert!((open.window.get_displayed_position() - 0.501).abs() < 0.0001);
    present_outputs(&open, &service, &|_| {}, vec![frame(12, 1, 0.3)]);
    assert!((open.window.get_displayed_position() - 0.501).abs() < 0.0001);
    present_outputs(&open, &service, &|_| {}, vec![frame(13, 2, 0.801)]);
    assert!((open.window.get_position() - 0.801).abs() < 0.0001);
    assert!((open.window.get_displayed_position() - 0.801).abs() < 0.0001);
    manager.close_all();
}

#[test]
fn refused_file_and_mode_changes_do_not_fence_the_current_text_context() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let mut initial = frame(10, 0, 0.2);
    if let ViewerOutput::Frame(frame) = &mut initial {
        frame.modes = vec![1, 2];
    }
    present_outputs(&open, &service, &|_| {}, vec![initial]);
    // Closed input deterministically exercises refusal without racing a real worker.
    open.session.close();
    open.window.invoke_seek(0.7);
    let context = open.window.get_position_context();
    open.window.invoke_change_mode(1);
    assert!(!dispatch_key(&open, "viewer.next-file"));
    assert_eq!(open.window.get_position_context(), context);
    assert_eq!(
        open.text_position.borrow().positions(),
        [f64::from(0.7_f32), 0.0]
    );
    present_outputs(&open, &service, &|_| {}, vec![frame(11, 1, 0.701)]);
    assert!((open.window.get_displayed_position() - 0.701).abs() < 0.0001);
    manager.close_all();
}

#[test]
fn accepted_later_navigation_cancels_an_unsubmitted_scrollbar_retry() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    open.text_position.borrow_mut().request(false, 0.9);
    // This models enqueue backpressure before the next accepted user gesture.
    open.text_position.borrow_mut().flush(|_| false);
    open.window.invoke_scroll(0.0, 20.0);
    open.text_position
        .borrow_mut()
        .flush(|_| panic!("superseded scrollbar retry crossed the wheel input"));
    assert_eq!(open.text_position.borrow().positions(), [0.2, 0.0]);
    assert!(matches!(
        submitted.borrow().as_slice(),
        [ViewerInput::Scroll { x: 0.0, y: 20.0 }]
    ));
    manager.close_all();
}

#[test]
fn raster_native_wheel_delivers_fractional_frames_between_notches() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        if let ViewerInput::Scroll { x, y } = input {
            received.borrow_mut().push([x, y]);
        }
        true
    });
    for (revision, mode) in [
        (10, 1),
        (11, 1),
        (12, 2),
        (13, 2),
        (14, 3),
        (15, 3),
        (16, 4),
        (17, 4),
        (18, VIEWER_MODE_IMAGE),
        (19, VIEWER_MODE_IMAGE),
    ] {
        let mut value = frame(revision, 0, 0.2);
        if let ViewerOutput::Frame(frame) = &mut value {
            frame.mode = mode;
            frame.modes = vec![mode];
            if mode == VIEWER_MODE_IMAGE {
                frame.image = Some(ViewerImageInfo {
                    surface: ViewerSurfaceId(99),
                    origin: [0.0, 500.0],
                    visible_source: [400.0, 240.0],
                    raster_scale: 1.0,
                    pan_sequence: 0,
                    width: 4000,
                    height: 3000,
                    zoom: 1.0,
                    fitted: false,
                    page: 1,
                    page_count: 3,
                });
            }
        }
        present_outputs(&open, &service, &|_| {}, vec![value]);
        submitted.borrow_mut().clear();
        let scale = f64::from(open.window.window().scale_factor());
        assert!(
            native_raster_notch(&open, -1.0),
            "raster mode {mode} bypassed the motion controller"
        );
        assert!(
            submitted.borrow().is_empty(),
            "a notch jumped before the first display frame"
        );
        raster_ticks(&open, 30);
        let steps = submitted.borrow();
        assert!(
            steps.len() >= 10,
            "mode {mode}: only {} movement frames",
            steps.len()
        );
        assert!(steps.iter().all(|step| step[1] > 0.0 && step[1] < 10.0));
        assert!(steps.iter().any(|step| step[1].fract() != 0.0));
        let total: f32 = steps.iter().map(|step| step[1]).sum();
        assert!(
            (total - 60.0 * scale as f32).abs() < 0.001,
            "lost distance: {total}"
        );
        image_presentation_tests::advance_time(Duration::from_millis(500));
    }
    manager.close_all();
}
fn native_raster_notch(open: &OpenWindow, lines: f32) -> bool {
    use slint::winit_030::winit::{
        dpi::PhysicalPosition,
        event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
    };
    let scale = f64::from(open.window.window().scale_factor());
    image_motion::observe(
        open,
        &WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(
                f64::from(open.window.get_content_left() + 20.0) * scale,
                f64::from(open.window.get_content_top() + 20.0) * scale,
            ),
        },
    );
    image_motion::observe(
        open,
        &WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, lines),
            phase: TouchPhase::Moved,
        },
    )
}

fn raster_ticks(open: &OpenWindow, count: usize) {
    for _ in 0..count {
        image_presentation_tests::advance_time(Duration::from_millis(8));
        image_motion::observe(
            open,
            &slint::winit_030::winit::event::WindowEvent::RedrawRequested,
        );
    }
}

#[test]
fn raster_pixel_input_and_scrollbar_cancel_the_previous_wheel_tail() {
    use slint::winit_030::winit::{
        dpi::PhysicalPosition,
        event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
    };
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    assert!(native_raster_notch(&open, -1.0));
    raster_ticks(&open, 2);
    submitted.borrow_mut().clear();
    assert!(
        !image_motion::observe(
            &open,
            &WindowEvent::MouseWheel {
                device_id: DeviceId::dummy(),
                delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -0.375)),
                phase: TouchPhase::Moved,
            }
        ),
        "precise pixels must keep their native delivery path"
    );
    open.window.invoke_scroll(0.0, 0.375);
    raster_ticks(&open, 30);
    assert!(
        matches!(
            submitted.borrow().as_slice(),
            [ViewerInput::Scroll { x: 0.0, y: 0.375 }]
        ),
        "native pixel input acquired an extra wheel tail"
    );

    assert!(native_raster_notch(&open, -1.0));
    raster_ticks(&open, 2);
    open.window.invoke_seek(0.7);
    let count = submitted.borrow().len();
    raster_ticks(&open, 30);
    assert_eq!(
        submitted.borrow().len(),
        count,
        "scrollbar drag must supersede wheel motion"
    );
    assert_eq!(
        open.window.get_position(),
        0.7,
        "absolute scrollbar tracking stays direct"
    );
    manager.close_all();
}

#[test]
fn raster_wheel_retries_a_full_queue_without_losing_its_last_pixels() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    let accepting = Rc::new(Cell::new(false));
    let accepted = accepting.clone();
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        if !accepted.get() {
            return false;
        }
        if let ViewerInput::Scroll { x, y } = input {
            received.borrow_mut().push([x, y]);
        }
        true
    });
    assert!(native_raster_notch(&open, -1.0));
    raster_ticks(&open, 3);
    assert!(submitted.borrow().is_empty());
    accepting.set(true);
    raster_ticks(&open, 40);
    let steps = submitted.borrow();
    assert!(steps.len() >= 10);
    assert!(steps.iter().all(|step| step[1] > 0.0 && step[1] < 10.0));
    let total: f32 = steps.iter().map(|step| step[1]).sum();
    assert!(
        (total - 60.0).abs() < 0.001,
        "queue backpressure lost wheel distance: {total}"
    );
    manager.close_all();
}

#[test]
fn raster_next_file_cancels_motion_even_when_the_new_file_keeps_text_mode() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    assert!(native_raster_notch(&open, -1.0));
    raster_ticks(&open, 2);
    assert!(dispatch_key(&open, "viewer.next-file"));
    let count = submitted.borrow().len();
    assert!(
        !native_raster_notch(&open, -1.0),
        "opening file has no usable motion context"
    );
    present_outputs(&open, &service, &|_| {}, vec![frame(11, 0, 0.25)]);
    raster_ticks(&open, 30);
    assert_eq!(
        submitted.borrow().len(),
        count,
        "old content restarted its wheel tail"
    );
    let mut next = frame(12, 0, 0.0);
    if let ViewerOutput::Frame(frame) = &mut next {
        frame.generation = 8;
    }
    present_outputs(&open, &service, &|_| {}, vec![next]);
    raster_ticks(&open, 30);
    assert_eq!(
        submitted.borrow().len(),
        count,
        "same-mode file inherited previous wheel movement"
    );
    assert!(native_raster_notch(&open, -1.0));
    raster_ticks(&open, 2);
    assert!(
        submitted.borrow().len() > count,
        "new file accepts its own wheel gesture"
    );
    manager.close_all();
}
#[test]
fn raster_keyboard_navigation_cancels_the_previous_wheel_tail() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    for action in [
        "viewer.up",
        "viewer.down",
        "viewer.page-up",
        "viewer.page-down",
    ] {
        assert!(native_raster_notch(&open, -1.0));
        raster_ticks(&open, 2);
        submitted.borrow_mut().clear();
        assert!(dispatch_key(&open, action));
        raster_ticks(&open, 30);
        assert_eq!(
            submitted.borrow().len(),
            1,
            "{action} inherited a wheel tail"
        );
        assert!(matches!(submitted.borrow()[0], ViewerInput::Scroll { .. }));
    }
    manager.close_all();
}

#[test]
fn raster_phased_line_gesture_keeps_native_momentum_without_an_added_tail() {
    use slint::winit_030::winit::event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent};
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    assert!(native_raster_notch(&open, -1.0));
    raster_ticks(&open, 2);
    submitted.borrow_mut().clear();
    for phase in [TouchPhase::Started, TouchPhase::Moved, TouchPhase::Ended] {
        assert!(
            !image_motion::observe(
                &open,
                &WindowEvent::MouseWheel {
                    device_id: DeviceId::dummy(),
                    delta: MouseScrollDelta::LineDelta(0.0, -0.1),
                    phase,
                }
            ),
            "phased {phase:?} acquired wheel easing"
        );
    }
    open.window.invoke_scroll(0.0, 1.25);
    raster_ticks(&open, 30);
    assert!(matches!(
        submitted.borrow().as_slice(),
        [ViewerInput::Scroll { x: 0.0, y: 1.25 }]
    ));
    assert!(
        native_raster_notch(&open, -1.0),
        "ended gesture left wheel input blocked"
    );
    manager.close_all();
}

#[test]
fn raster_coast_survives_pointer_motion_but_another_scroll_target_cancels_it() {
    use slint::winit_030::winit::{
        dpi::PhysicalPosition,
        event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
    };
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    assert!(native_raster_notch(&open, -1.0));
    raster_ticks(&open, 2);
    image_motion::observe(
        &open,
        &WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(1.0, 1.0),
        },
    );
    let count = submitted.borrow().len();
    raster_ticks(&open, 2);
    assert!(
        submitted.borrow().len() > count,
        "moving the pointer alone stopped deceleration"
    );
    assert!(!image_motion::observe(
        &open,
        &WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -1.0),
            phase: TouchPhase::Moved,
        }
    ));
    let count = submitted.borrow().len();
    raster_ticks(&open, 30);
    assert_eq!(
        submitted.borrow().len(),
        count,
        "another scroll target kept moving old content"
    );
    manager.close_all();
}

#[test]
fn source_opening_keeps_first_glyph_geometry_when_its_toolbar_appears() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    for (scale, retained) in [(1.0, true), (1.5, true), (1.0, false), (1.5, false)] {
        open.window
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: scale,
            });
        open.window
            .window()
            .set_size(slint::LogicalSize::new(900.0, 600.0));
        open.window.set_image_mode(false);
        open.window.set_source_mode(false);
        image_presentation_tests::bitmap();
        let initial = window_geometry(&open.window);
        let revision = open.frame.get() + 1;
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![source_opening_frame(revision, initial, retained)],
        );
        let first = source_opening_marks();
        if retained {
            let selected = raster_presentation::select(&open, 25.0, 25.0, false).unwrap();
            assert!(
                matches!(selected, ViewerInput::Raster {
                input: ViewerRasterInput::Select { y, .. }, ..
            } if (y - 25.0 * scale).abs() < 0.1),
                "initial selection disagrees with raster pixels"
            );
            let ViewerOutput::Frame(mut acknowledgement) =
                source_opening_frame(revision + 1, initial, retained)
            else {
                unreachable!()
            };
            acknowledgement.raster.as_mut().unwrap().surface = ViewerSurfaceId(1200 + revision);
            present_outputs(
                &open,
                &service,
                &|_| {},
                vec![ViewerOutput::Frame(acknowledgement)],
            );
            assert_eq!(
                first,
                source_opening_marks(),
                "same-surface acknowledgement stretched source pixels"
            );
        } else {
            present_outputs(
                &open,
                &service,
                &|_| {},
                vec![source_opening_frame(revision + 1, initial, true)],
            );
            assert_eq!(
                first,
                source_opening_marks(),
                "retained activation stretched the initial source pixels"
            );
        }
        let settled = window_geometry(&open.window);
        assert!(
            settled.height < initial.height,
            "source toolbar did not change the canvas"
        );
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![source_opening_frame(revision + 2, settled, retained)],
        );
        let second = source_opening_marks();
        assert_eq!(
            first, second,
            "first source frame changed glyph size or position at scale {scale}"
        );
    }
    manager.close_all();
}

fn source_opening_frame(revision: u64, geometry: ViewerGeometry, retained: bool) -> ViewerOutput {
    let ViewerOutput::Frame(mut frame) = frame(revision, 0, 0.0) else {
        unreachable!()
    };
    let gutter_width = 36;
    let pixels = |width: u32, gutter: bool| {
        let mut pixels = vec![0; width as usize * geometry.height as usize * 4];
        for y in 0..geometry.height {
            for x in 0..width {
                let color = if (8..28).contains(&y) && (4..12).contains(&x) && gutter {
                    [20, 230, 245, 255]
                } else if (8..28).contains(&y) && (30..42).contains(&x) && !gutter {
                    [250, 30, 20, 255]
                } else if (200..220).contains(&y) && (30..42).contains(&x) && !gutter {
                    [45, 70, 240, 255]
                } else {
                    [0, 0, 0, 255]
                };
                let index = ((y * width + x) * 4) as usize;
                pixels[index..index + 4].copy_from_slice(&color);
            }
        }
        Arc::from(pixels)
    };
    frame.mode = 2;
    frame.modes = vec![1, 2];
    frame.width = geometry.width - gutter_width;
    frame.height = geometry.height;
    frame.pixels = pixels(frame.width, false);
    frame.raster = Some(ViewerRasterInfo {
        surface: ViewerSurfaceId(1200 + revision),
        pixel_id: 1,
        origin: [0.0; 2],
        raster_origin: [0.0; 2],
        viewport: [geometry.width, geometry.height],
        coverage_min: [0.0; 2],
        coverage_max: [0.0; 2],
        limit_min: [Some(0.0); 2],
        limit_max: [Some(0.0); 2],
        pan_sequence: 0,
        scroll_per_pixel: [0.0; 2],
        scroll_map: None,
        settled: true,
        gutter: Some(ViewerRasterGutter {
            width: gutter_width,
            height: geometry.height,
            pixels: pixels(gutter_width, true),
        }),
    });
    if !retained {
        let gutter = frame.raster.take().unwrap().gutter.unwrap();
        let mut combined =
            Vec::with_capacity(geometry.width as usize * geometry.height as usize * 4);
        for row in 0..geometry.height as usize {
            let gutter_stride = gutter_width as usize * 4;
            let body_stride = frame.width as usize * 4;
            combined
                .extend_from_slice(&gutter.pixels[row * gutter_stride..(row + 1) * gutter_stride]);
            combined.extend_from_slice(&frame.pixels[row * body_stride..(row + 1) * body_stride]);
        }
        frame.width = geometry.width;
        frame.pixels = combined.into();
    }
    ViewerOutput::Frame(frame)
}

fn source_opening_marks() -> [[usize; 4]; 3] {
    let (size, pixels) = image_presentation_tests::bitmap();
    [[250, 30, 20], [45, 70, 240], [20, 230, 245]].map(|color| {
        let mut bounds = [usize::MAX, usize::MAX, 0, 0];
        let mut found = false;
        for (index, pixel) in pixels.iter().enumerate() {
            if [pixel.r, pixel.g, pixel.b] == color {
                let x = index % size.width as usize;
                let y = index / size.width as usize;
                bounds[0] = bounds[0].min(x);
                bounds[1] = bounds[1].min(y);
                bounds[2] = bounds[2].max(x);
                bounds[3] = bounds[3].max(y);
                found = true;
            }
        }
        assert!(found, "source glyph marker was not visible");
        bounds
    })
}
