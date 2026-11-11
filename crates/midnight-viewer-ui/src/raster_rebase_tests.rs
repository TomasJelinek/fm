//! Rebase handoffs preserve the displayed camera and undelivered wheel intent.
use super::*;

fn frame(open: &OpenWindow, revision: u64, surface: u64, ack: Option<u64>) -> ViewerOutput {
    let ViewerOutput::Frame(mut frame) = text_presentation_tests::frame(revision, 0, 0.2) else {
        unreachable!()
    };
    frame.raster_rebase = ack;
    frame.width = open.window.get_content_width() as u32;
    frame.height = open.window.get_content_height() as u32;
    frame.pixels = vec![255; frame.width as usize * frame.height as usize * 4].into();
    if surface != 0 {
        frame.width = 1000;
        frame.height = 1000;
        frame.pixels = vec![255; 4_000_000].into();
        frame.raster = Some(ViewerRasterInfo {
            surface: ViewerSurfaceId(surface),
            pixel_id: revision,
            origin: [0.0, 200.0],
            raster_origin: [0.0; 2],
            viewport: [
                open.window.get_content_width() as u32,
                open.window.get_content_height() as u32,
            ],
            coverage_min: [0.0; 2],
            coverage_max: [100.0, if surface == 90 { 200.0 } else { 600.0 }],
            limit_min: [Some(0.0); 2],
            limit_max: [None; 2],
            pan_sequence: 0,
            scroll_per_pixel: [0.001; 2],
            settled: true,
            gutter: None,
        });
    }
    ViewerOutput::Frame(frame)
}

fn present(open: &OpenWindow, service: &Service, value: ViewerOutput) {
    present_outputs(open, service, &|_| {}, vec![value]);
}
fn start(open: &OpenWindow, service: &Service) {
    open.timer.stop();
    open.image_redraw.stop();
    present(open, service, frame(open, 10, 90, None));
}
fn rebase(open: &OpenWindow, service: &Service, transition: u64) {
    present(
        open,
        service,
        ViewerOutput::RasterRebase {
            surface: ViewerSurfaceId(90),
            transition,
        },
    );
}
fn capture(open: &OpenWindow) -> Rc<RefCell<Vec<ViewerInput>>> {
    let received = Rc::new(RefCell::new(Vec::new()));
    let sink = received.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        sink.borrow_mut().push(input);
        true
    });
    received
}
fn scroll_y(received: &RefCell<Vec<ViewerInput>>) -> Vec<f32> {
    received
        .borrow()
        .iter()
        .filter_map(|input| match input {
            ViewerInput::Scroll { y, .. } => Some(*y),
            _ => None,
        })
        .collect()
}
fn advance(open: &OpenWindow, ms: u64) {
    image_presentation_tests::advance_time(Duration::from_millis(ms));
    raster_motion::step(open);
}

#[test]
fn raster_rebase_to_legacy_replies_with_displayed_camera_and_resumes_only_remaining_coast() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let received = capture(&open);
    start(&open, &service);
    assert!(raster_motion::push(&open, 0.0, 200.0));
    advance(&open, 16);
    let displayed = open.retained_raster.borrow().camera.origin;
    assert_eq!(displayed, [0.0, 200.0]);
    rebase(&open, &service, 1);
    assert!(
        received.borrow().iter().any(|input| matches!(input,
        ViewerInput::RasterRebase { transition: 1, origin } if *origin == displayed)),
        "reply must acknowledge the camera actually displayed, not its speculative target"
    );
    assert!(
        open.window.get_retained_raster(),
        "old pixels remain visible while waiting"
    );
    assert!(
        !open.raster_redraw.running(),
        "sent handoff must not poll/redraw while waiting"
    );
    advance(&open, 1000);
    assert!(scroll_y(&received).is_empty());
    let corrected = frame(&open, 11, 0, Some(1));
    let ViewerOutput::Frame(value) = &corrected else {
        unreachable!()
    };
    let effective_scale = value.height as f32 / open.window.get_content_height();
    present(&open, &service, corrected);
    assert!(!open.window.get_retained_raster());
    assert!(
        scroll_y(&received).is_empty(),
        "waiting cannot become a catch-up jump"
    );
    advance(&open, 16);
    let first = scroll_y(&received)[0];
    assert!(first > 0.0 && first < 60.0, "first resumed frame {first}");
    for _ in 0..20 {
        advance(&open, 16);
    }
    let sum: f32 = scroll_y(&received).into_iter().sum();
    assert!(
        (sum - 200.0 * effective_scale).abs() < 0.001,
        "lost wheel distance: {sum}"
    );
    manager.close_all();
}

#[test]
fn raster_rebase_retained_resume_preserves_precise_input_and_reversal_while_waiting() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let received = capture(&open);
    start(&open, &service);
    open.window.invoke_scroll(0.0, 200.0);
    rebase(&open, &service, 1);
    let old = [
        open.window.get_raster_offset_x(),
        open.window.get_raster_offset_y(),
    ];
    open.window.invoke_scroll(0.0, 30.0);
    open.window.invoke_scroll(0.0, -12.5);
    assert_eq!(
        [
            open.window.get_raster_offset_x(),
            open.window.get_raster_offset_y()
        ],
        old
    );
    assert!(
        scroll_y(&received).is_empty(),
        "waiting pixels must not scroll the speculative page"
    );
    let ViewerOutput::Frame(corrected) = frame(&open, 11, 91, Some(1)) else {
        unreachable!()
    };
    let ratio = corrected.raster.as_ref().unwrap().viewport[1] as f64
        / f64::from(open.window.get_content_height());
    present(&open, &service, ViewerOutput::Frame(corrected));
    assert!(
        (open.retained_raster.borrow().camera.origin[1] - (200.0 - 12.5 * ratio)).abs() < 0.0001
    );
    assert!(scroll_y(&received).is_empty());
    manager.close_all();
}

#[test]
fn raster_rebase_queue_retries_once_until_accepted_and_freezes_old_frame_context() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let accept = Rc::new(Cell::new(false));
    let attempts = Rc::new(Cell::new(0));
    let a = accept.clone();
    let n = attempts.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        if matches!(input, ViewerInput::RasterRebase { .. }) {
            n.set(n.get() + 1);
            a.get()
        } else {
            true
        }
    });
    start(&open, &service);
    rebase(&open, &service, 3);
    assert_eq!(attempts.get(), 1);
    assert!(open.raster_redraw.running());
    present(&open, &service, frame(&open, 11, 0, None));
    assert_eq!(
        open.frame.get(),
        10,
        "unacknowledged frame must not mutate presentation or TextPosition"
    );
    assert!(open.window.get_retained_raster());
    advance(&open, 16);
    assert_eq!(attempts.get(), 2);
    accept.set(true);
    advance(&open, 16);
    assert_eq!(attempts.get(), 3);
    assert!(!open.raster_redraw.running());
    advance(&open, 1000);
    rebase(&open, &service, 3);
    assert_eq!(
        attempts.get(),
        3,
        "duplicate output cannot restart the handshake"
    );
    present(&open, &service, frame(&open, 12, 91, Some(3)));
    assert_eq!(open.frame.get(), 12);
    manager.close_all();
}

#[test]
fn raster_rebase_ack_is_sticky_but_cancelled_and_unrelated_acks_cannot_replace_pixels() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    capture(&open);
    start(&open, &service);
    rebase(&open, &service, 2);
    present(&open, &service, frame(&open, 11, 91, Some(1)));
    assert_eq!(open.frame.get(), 10);
    present(&open, &service, frame(&open, 12, 91, Some(2)));
    assert_eq!(open.frame.get(), 12);
    present(&open, &service, frame(&open, 13, 91, Some(2)));
    assert_eq!(
        open.frame.get(),
        13,
        "coalesced metadata may repeat the sticky ack"
    );
    assert!(open.enqueue(ViewerInput::Seek(0.8)));
    present(&open, &service, frame(&open, 14, 92, Some(2)));
    assert_eq!(
        open.frame.get(),
        13,
        "explicit navigation invalidates the old ack"
    );
    present(&open, &service, frame(&open, 15, 93, None));
    assert_eq!(open.frame.get(), 15);
    manager.close_all();
}

#[test]
fn raster_rebase_pending_selection_is_blocked_and_mouse_cancel_only_discards_motion() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let received = capture(&open);
    start(&open, &service);
    assert!(raster_motion::push(&open, 0.0, 200.0));
    rebase(&open, &service, 1);
    received.borrow_mut().clear();
    open.window.invoke_select_point(10.0, 10.0, false);
    assert!(
        received.borrow().is_empty(),
        "point selection cannot target speculative legacy content"
    );
    assert!(dispatch_key(&open, "viewer.select-page"));
    assert!(dispatch_key(&open, "viewer.copy"));
    assert!(
        received.borrow().is_empty(),
        "selection must not fall through to speculative legacy layout"
    );
    present(&open, &service, frame(&open, 11, 0, Some(1)));
    for _ in 0..20 {
        advance(&open, 16);
    }
    assert!(
        scroll_y(&received).is_empty(),
        "pointer cancellation discards coast but preserves correction"
    );
    assert_eq!(open.frame.get(), 11);
    open.window.invoke_select_point(10.0, 10.0, false);
    assert!(
        received
            .borrow()
            .iter()
            .any(|input| matches!(input, ViewerInput::Select { frame: 11, .. })),
        "legacy selection resumes only after corrected presentation"
    );
    manager.close_all();
}

#[test]
fn raster_rebase_failure_releases_wait_and_rejects_late_success_ack() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let received = capture(&open);
    start(&open, &service);
    open.window.invoke_scroll(0.0, 200.0);
    rebase(&open, &service, 4);
    present(
        &open,
        &service,
        ViewerOutput::RasterUnavailable {
            surface: ViewerSurfaceId(90),
            message: "guard read failed".into(),
        },
    );
    assert_eq!(open.window.get_status().as_str(), "guard read failed");
    present(&open, &service, frame(&open, 11, 91, Some(4)));
    assert_eq!(open.frame.get(), 10);
    present(&open, &service, frame(&open, 12, 0, None));
    assert_eq!(open.frame.get(), 12);
    advance(&open, 1000);
    assert!(scroll_y(&received).is_empty());
    manager.close_all();
}

#[test]
fn raster_rebase_keyboard_relative_scroll_joins_pending_intent() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let received = capture(&open);
    start(&open, &service);
    rebase(&open, &service, 1);
    assert!(dispatch_key(&open, "viewer.down"));
    assert!(
        scroll_y(&received).is_empty(),
        "keyboard cannot advance the speculative origin"
    );
    present(&open, &service, frame(&open, 11, 0, Some(1)));
    assert!(
        !scroll_y(&received).is_empty(),
        "direct keyboard intent resumes after correction"
    );
    manager.close_all();
}

#[test]
fn raster_rebase_legacy_resume_uses_bounded_effective_dpi_in_each_axis() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let received = capture(&open);
    start(&open, &service);
    rebase(&open, &service, 1);
    open.window.invoke_scroll(12.5, 100.25);
    let ViewerOutput::Frame(mut corrected) = frame(&open, 11, 0, Some(1)) else {
        unreachable!()
    };
    corrected.width /= 2;
    corrected.height /= 3;
    corrected.pixels = vec![255; corrected.width as usize * corrected.height as usize * 4].into();
    let expected = [
        12.5 * corrected.width as f32 / open.window.get_content_width(),
        100.25 * corrected.height as f32 / open.window.get_content_height(),
    ];
    present(&open, &service, ViewerOutput::Frame(corrected));
    let movement: [f32; 2] = received
        .borrow()
        .iter()
        .fold([0.0; 2], |sum, input| match input {
            ViewerInput::Scroll { x, y } => [sum[0] + x, sum[1] + y],
            _ => sum,
        });
    assert!((movement[0] - expected[0]).abs() < 0.0001);
    assert!((movement[1] - expected[1]).abs() < 0.0001);
    assert!(
        movement[1] < 40.0,
        "native DPI would overdeliver the pending logical pixels"
    );
    manager.close_all();
}

#[test]
fn raster_rebase_native_detents_are_queued_until_a_corrected_frame_is_presented() {
    use slint::winit_030::winit::{
        dpi::PhysicalPosition,
        event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
    };
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let received = capture(&open);
    start(&open, &service);
    rebase(&open, &service, 1);
    let scale = f64::from(open.window.window().scale_factor());
    image_motion::observe(
        &open,
        &WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(
                f64::from(open.window.get_content_left() + 20.0) * scale,
                f64::from(open.window.get_content_top() + 20.0) * scale,
            ),
        },
    );
    assert!(image_motion::observe(
        &open,
        &WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -1.0),
            phase: TouchPhase::Moved,
        }
    ));
    advance(&open, 1000);
    assert!(scroll_y(&received).is_empty());
    present(&open, &service, frame(&open, 11, 0, Some(1)));
    assert!(scroll_y(&received).is_empty());
    advance(&open, 16);
    assert!(scroll_y(&received)[0] > 0.0 && scroll_y(&received)[0] < 20.0);
    manager.close_all();
}

#[test]
fn raster_rebase_invalid_corrected_pixels_do_not_commit_or_replay_pending_intent() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    capture(&open);
    start(&open, &service);
    rebase(&open, &service, 1);
    open.window.invoke_scroll(0.0, 25.0);
    let ViewerOutput::Frame(mut bad) = frame(&open, 11, 91, Some(1)) else {
        unreachable!()
    };
    bad.pixels = vec![0; 4].into();
    present(&open, &service, ViewerOutput::Frame(bad));
    assert_eq!(open.frame.get(), 10);
    let corrected = frame(&open, 12, 91, Some(1));
    let ViewerOutput::Frame(value) = &corrected else {
        unreachable!()
    };
    let ratio = value.raster.as_ref().unwrap().viewport[1] as f64
        / f64::from(open.window.get_content_height());
    present(&open, &service, corrected);
    let moved = open.retained_raster.borrow().camera.origin;
    assert!((moved[1] - (200.0 + 25.0 * ratio)).abs() < 0.0001);
    present(&open, &service, frame(&open, 13, 91, Some(1)));
    assert_eq!(
        open.retained_raster.borrow().camera.origin,
        moved,
        "duplicate ack cannot replay input"
    );
    manager.close_all();
}

#[test]
fn raster_rebase_font_restore_and_wrap_invalidate_pending_acknowledgements() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    capture(&open);
    start(&open, &service);
    rebase(&open, &service, 1);
    assert!(accept_preference(&open, ViewerPreferenceChange::Wrap(true)));
    present(&open, &service, frame(&open, 11, 91, Some(1)));
    assert_eq!(open.frame.get(), 10);
    present(&open, &service, frame(&open, 12, 90, None));
    // Recreate a fresh context: the previous suspended surface must not be reused.
    present(&open, &service, frame(&open, 13, 92, None));
    present(
        &open,
        &service,
        ViewerOutput::RasterRebase {
            surface: ViewerSurfaceId(92),
            transition: 2,
        },
    );
    let mut preferences = (*open.session.preferences()).clone();
    preferences.text_font.size += 1;
    manager.restore_preferences(std::sync::Arc::new(preferences));
    present(&open, &service, frame(&open, 14, 93, Some(2)));
    assert_eq!(open.frame.get(), 13);
    manager.close_all();
}

#[test]
fn raster_rebase_resumed_precise_legacy_tail_survives_backpressure_and_reverses() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let accept = Rc::new(Cell::new(false));
    let accepted = Rc::new(RefCell::new(Vec::new()));
    let allowed = accept.clone();
    let sink = accepted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        if matches!(input, ViewerInput::Scroll { .. }) && !allowed.get() {
            return false;
        }
        sink.borrow_mut().push(input);
        true
    });
    for (index, (extra, expected)) in [(7.5, 32.5), (-7.5, -7.5)].into_iter().enumerate() {
        let revision = 10 + index as u64 * 10;
        let transition = 1 + index as u64;
        accept.set(false);
        accepted.borrow_mut().clear();
        present(&open, &service, frame(&open, revision, 90, None));
        open.window.invoke_scroll(0.0, 25.0);
        rebase(&open, &service, transition);
        let corrected = frame(&open, revision + 1, 0, Some(transition));
        let ViewerOutput::Frame(value) = &corrected else {
            unreachable!()
        };
        let effective_scale = value.height as f32 / open.window.get_content_height();
        present(&open, &service, corrected);
        assert!(
            scroll_y(&accepted).is_empty(),
            "first precise tail is queued behind backpressure"
        );
        open.window.invoke_scroll(0.0, extra);
        assert!(scroll_y(&accepted).is_empty());
        accept.set(true);
        advance(&open, 16);
        let sum: f32 = scroll_y(&accepted).iter().sum();
        assert!(
            (sum - expected * effective_scale).abs() < 0.0001,
            "queued precise tail lost/replayed at reversal {extra}: {sum}"
        );
        advance(&open, 16);
        assert_eq!(
            scroll_y(&accepted).len(),
            1,
            "accepted direct pixels must not replay"
        );
    }
    manager.close_all();
}
