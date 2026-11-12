use super::*;

fn info() -> ViewerRasterInfo {
    ViewerRasterInfo {
        surface: ViewerSurfaceId(7),
        pixel_id: 1,
        origin: [100.0, 400.0],
        raster_origin: [0.0, 144.0],
        viewport: [800, 600],
        coverage_min: [0.0, 144.0],
        coverage_max: [356.0, 656.0],
        limit_min: [Some(0.0), Some(0.0)],
        limit_max: [Some(2000.0), None],
        pan_sequence: 0,
        settled: true,
        scroll_per_pixel: [0.0005, 0.00001],
        scroll_map: None,
        gutter: None,
    }
}

#[test]
fn retained_pixels_move_fractionally_without_waiting_for_a_worker_reply() {
    let mut camera = Camera::default();
    assert!(camera.receive(&info(), [2.25; 2]));
    assert!(camera.push(3.5, 7.25, Duration::ZERO, true));
    assert!(camera.tick(Duration::ZERO));
    assert_eq!(camera.origin, [107.875, 416.3125]);
    assert!(!camera.active());
    assert_eq!(camera.info().unwrap().pixel_id, 1);
}

#[test]
fn wheel_is_fractional_at_every_frame_and_ends_at_the_exact_requested_distance() {
    let mut camera = Camera::default();
    camera.receive(&info(), [2.25; 2]);
    camera.push(0.0, 60.0, Duration::ZERO, false);
    let mut previous = camera.origin[1];
    let mut advances = 0;
    for frame in 1..=25 {
        camera.tick(Duration::from_millis(frame * 8));
        let delta = camera.origin[1] - previous;
        assert!((0.0..12.0).contains(&delta), "large physical step {delta}");
        advances += usize::from(delta > 0.0);
        previous = camera.origin[1];
    }
    assert!(advances >= 24);
    assert!((camera.origin[1] - 535.0).abs() < 0.001);
    assert!(!camera.active());
}

#[test]
fn a_missing_frontier_keeps_motion_and_resumes_when_coverage_arrives() {
    let mut camera = Camera::default();
    let mut next = info();
    next.coverage_max[1] = 410.0;
    camera.receive(&next, [1.0; 2]);
    camera.push(0.0, 60.0, Duration::ZERO, true);
    camera.tick(Duration::ZERO);
    assert_eq!(camera.origin[1], 410.0);
    assert!(camera.active());
    assert!(!camera.tick(Duration::from_millis(100)));
    let (_, sequence, target, settled) = camera.request().unwrap();
    assert_eq!(target[1], 460.0);
    assert!(!settled);
    next.origin = target;
    next.pan_sequence = sequence;
    next.pixel_id += 1;
    next.coverage_max[1] = 716.0;
    assert!(camera.receive(&next, [1.0; 2]));
    assert_eq!(
        camera.origin[1], 410.0,
        "acknowledgement must not jump the camera"
    );
    assert!(camera.tick(Duration::from_millis(108)));
    assert_eq!(camera.origin[1], 460.0);
    assert!(!camera.active());
    assert!(camera.request().unwrap().3, "send exact final phase");
}

#[test]
fn reversal_and_late_acknowledgements_never_snap_back_or_replace_visible_coverage() {
    let mut camera = Camera::default();
    let mut next = info();
    camera.receive(&next, [1.0; 2]);
    camera.push(0.0, 30.0, Duration::ZERO, true);
    camera.tick(Duration::ZERO);
    let request = camera.request().unwrap();
    camera.push(0.0, -5.5, Duration::from_millis(1), true);
    camera.tick(Duration::from_millis(1));
    next.origin = request.2;
    next.pan_sequence = request.1;
    assert!(camera.receive(&next, [1.0; 2]));
    assert_eq!(camera.origin[1], 424.5);
    next.coverage_min[1] = 500.0;
    assert!(!camera.receive(&next, [1.0; 2]));
    assert_eq!(camera.origin[1], 424.5);
}

#[test]
fn known_eof_stops_only_that_axis_and_context_changes_discard_old_motion() {
    let mut camera = Camera::default();
    let mut next = info();
    next.limit_max[1] = Some(405.0);
    camera.receive(&next, [1.0; 2]);
    camera.push(20.0, 60.0, Duration::ZERO, true);
    camera.tick(Duration::ZERO);
    assert_eq!(camera.origin, [120.0, 405.0]);
    assert!(!camera.active());
    camera.push(0.0, -60.0, Duration::from_secs(1), false);
    camera.suspend();
    assert!(!camera.receive(&next, [1.0; 2]));
    assert!(!camera.push(0.0, 10.0, Duration::from_secs(2), false));
    next.surface = ViewerSurfaceId(8);
    next.origin = [0.0, 200.0];
    assert!(camera.receive(&next, [1.0; 2]));
    assert_eq!(camera.origin, next.origin);
    assert!(!camera.active());
}

#[test]
fn backpressure_retries_absolute_intent_and_invalid_metadata_never_changes_display() {
    let mut camera = Camera::default();
    camera.receive(&info(), [1.0; 2]);
    camera.push(0.0, 30.0, Duration::ZERO, true);
    camera.tick(Duration::ZERO);
    let first = camera.request().unwrap();
    assert!(camera.request().is_none());
    camera.retry();
    let retry = camera.request().unwrap();
    assert_eq!((first.0, first.2, first.3), (retry.0, retry.2, retry.3));
    assert!(retry.1 > first.1);
    let mut invalid = info();
    invalid.coverage_max[1] = f64::NAN;
    assert!(!camera.receive(&invalid, [1.0; 2]));
    assert!(!camera.push(0.0, f32::NAN, Duration::ZERO, true));
    assert_eq!(camera.origin[1], 430.0);
}

#[test]
fn delayed_frontier_resumes_with_small_frames_instead_of_spending_unseen_animation_time() {
    let mut camera = Camera::default();
    let mut next = info();
    next.coverage_max[1] = 405.0;
    camera.receive(&next, [1.0; 2]);
    camera.push(0.0, 60.0, Duration::ZERO, false);
    for frame in 1..=60 {
        camera.tick(Duration::from_millis(frame * 16));
    }
    assert_eq!(camera.origin[1], 405.0);
    next.coverage_max[1] = 700.0;
    assert!(camera.receive(&next, [1.0; 2]));
    let mut before = camera.origin[1];
    for frame in 61..=80 {
        camera.tick(Duration::from_millis(frame * 16));
        let step = camera.origin[1] - before;
        assert!(step < 10.0, "frontier refill jumped {step} pixels");
        before = camera.origin[1];
    }
    assert!((camera.origin[1] - 460.0).abs() < 0.001);
    assert!(!camera.active());
}

#[test]
fn first_redraw_after_refill_advances_a_bounded_step_without_waiting_an_extra_frame() {
    for direction in [-1.0_f32, 1.0] {
        for scale in [1.0, 2.25] {
            for (pause_ms, redraw_during_wait) in [(32, true), (960, true), (960, false)] {
                let mut camera = Camera::default();
                let mut next = info();
                let boundary = 400.0 + f64::from(direction) * 5.0 * scale;
                if direction > 0.0 {
                    next.coverage_max[1] = boundary;
                } else {
                    next.coverage_min[1] = boundary;
                }
                assert!(camera.receive(&next, [scale; 2]));
                assert!(camera.push(0.0, direction * 60.0, Duration::ZERO, false));
                if redraw_during_wait {
                    for millis in (16..=pause_ms).step_by(16) {
                        camera.tick(Duration::from_millis(millis));
                    }
                } else {
                    camera.tick(Duration::from_millis(16));
                }
                assert_eq!(camera.origin[1], boundary);
                next.coverage_min[1] = 0.0;
                next.coverage_max[1] = 1000.0;
                assert!(camera.receive(&next, [scale; 2]));
                let now = Duration::from_millis(pause_ms + 16);
                assert!(camera.tick(now), "the first covered redraw must move");
                let step = (camera.origin[1] - boundary) * f64::from(direction) / scale;
                assert!(step > 0.0 && step < 10.0, "unbounded resume step: {step}");
                assert!(
                    !camera.tick(now),
                    "the same timestamp must not replay movement"
                );
                for frame in 1..=20 {
                    camera.tick(now + Duration::from_millis(frame * 16));
                }
                let target = 400.0 + f64::from(direction) * 60.0 * scale;
                assert!((camera.origin[1] - target).abs() < 0.001);
                assert!(!camera.active());
            }
        }
    }
}

#[test]
fn cancelling_a_moving_camera_keeps_an_exact_settle_request_alive_until_submission() {
    let mut camera = Camera::default();
    camera.receive(&info(), [1.0; 2]);
    camera.push(0.0, 60.0, Duration::ZERO, false);
    camera.tick(Duration::from_millis(16));
    camera.request().unwrap();
    let displayed = camera.origin;
    camera.cancel();
    assert!(
        camera.active(),
        "final glyph phase must still wake the presentation timer"
    );
    let (_, _, target, settled) = camera.request().unwrap();
    assert_eq!(target, displayed);
    assert!(settled);
    assert!(!camera.active());
}

#[test]
fn exact_latest_settle_reconciles_backend_fractional_precision_without_an_upload_loop() {
    let mut camera = Camera::default();
    let mut ack = info();
    camera.receive(&ack, [1.75; 2]);
    camera.push(0.0, 0.1, Duration::ZERO, true);
    camera.tick(Duration::ZERO);
    let (_, sequence, target, settled) = camera.request().unwrap();
    assert!(settled);
    ack.pan_sequence = sequence;
    ack.origin = target.map(|value| f64::from(value as f32));
    assert!(camera.receive(&ack, [1.75; 2]));
    assert_eq!(camera.origin, ack.origin);
    assert!(camera.request().is_none());
}

#[test]
fn a_narrower_fragment_clamps_horizontal_camera_before_checking_its_new_coverage() {
    let mut camera = Camera::default();
    let mut next = info();
    camera.receive(&next, [1.0; 2]);
    camera.push(80.0, 0.0, Duration::ZERO, true);
    camera.tick(Duration::ZERO);
    assert_eq!(camera.origin[0], 180.0);
    next.pixel_id = 2;
    next.origin[0] = 20.0;
    next.coverage_max[0] = 20.0;
    next.limit_max[0] = Some(20.0);
    assert!(
        camera.receive(&next, [1.0; 2]),
        "narrow continuation must not leave the old page stuck"
    );
    assert_eq!(camera.origin[0], 20.0);
}

#[test]
fn short_retained_coverage_limits_prefetch_without_losing_forward_or_backward_intent() {
    for direction in [-1.0_f32, 1.0] {
        let mut next = info();
        next.coverage_min[1] = 400.0;
        next.coverage_max[1] = 480.0;
        next.origin[1] = if direction > 0.0 { 480.0 } else { 400.0 };
        let mut camera = Camera::default();
        assert!(camera.receive(&next, [1.0; 2]));
        assert!(camera.push(0.0, direction * 200.0, Duration::ZERO, true));
        camera.tick(Duration::ZERO);
        let (_, sequence, target, settled) = camera.request().unwrap();
        let expected = next.origin[1] + f64::from(direction) * 40.0;
        assert_eq!(
            target[1], expected,
            "prefetch must retain half of this short coverage"
        );
        assert!(!settled);
        next.pan_sequence = sequence;
        next.pixel_id += 1;
        next.origin = target;
        if direction > 0.0 {
            next.coverage_max[1] = 800.0;
        } else {
            next.coverage_min[1] = 100.0;
        }
        assert!(camera.receive(&next, [1.0; 2]));
        assert!(camera.tick(Duration::from_millis(1)));
        let expected = if direction > 0.0 { 680.0 } else { 200.0 };
        assert_eq!(
            camera.origin[1], expected,
            "remaining precise input must not be lost"
        );
        assert!(!camera.active());
    }
}

#[test]
fn retained_scrollbar_maps_partial_page_positions_and_preserves_the_other_axis() {
    let mut camera = Camera::default();
    camera.receive(&info(), [2.25; 2]);
    assert!(camera.seek(1, 0.30125, 0.3, false));
    assert!((camera.origin[1] - 525.0).abs() < 1e-8);
    assert_eq!(camera.origin[0], 100.0);
    assert!(camera.seek(0, 0.175, 0.1, false));
    assert!((camera.origin[0] - 250.0).abs() < 1e-8);
    assert!((camera.origin[1] - 525.0).abs() < 1e-8);
    let (_, _, target, settled) = camera.request().unwrap();
    assert_eq!(target, camera.origin);
    assert!(settled);
}

#[test]
fn retained_scrollbar_refuses_uncovered_nonfinite_and_known_edge_targets() {
    let mut camera = Camera::default();
    camera.receive(&info(), [1.0; 2]);
    for target in [0.0, 1.0, -0.1, 1.1, f64::NAN, f64::INFINITY] {
        assert!(!camera.seek(1, target, 0.3, false));
        assert_eq!(camera.origin, info().origin);
    }
    let mut limited = info();
    limited.limit_max[1] = Some(450.0);
    camera.receive(&limited, [1.0; 2]);
    assert!(
        !camera.seek(1, 0.301, 0.3, false),
        "covered pixels beyond a known edge are not a valid target"
    );
    assert!(!camera.seek(2, 0.3, 0.3, false));
    limited.scroll_per_pixel[1] = 0.0;
    camera.receive(&limited, [1.0; 2]);
    assert!(
        !camera.seek(1, 0.3, 0.3, false),
        "zero slope does not define a normalized inverse"
    );
}

#[test]
fn retained_scrollbar_reversal_replaces_backpressured_target_and_cancels_old_coast() {
    let mut camera = Camera::default();
    camera.receive(&info(), [1.0; 2]);
    camera.push(0.0, 60.0, Duration::ZERO, false);
    assert!(camera.seek(1, 0.301, 0.3, false));
    let first = camera.request().unwrap();
    camera.retry();
    assert!(camera.seek(1, 0.2995, 0.3, false));
    let last = camera.request().unwrap();
    assert!(last.1 > first.1);
    assert!((last.2[1] - 350.0).abs() < 1e-8);
    assert!(
        !camera.tick(Duration::from_secs(1)),
        "old coast must not resume after the drag"
    );
    assert!(camera.request().is_none());
}

#[test]
fn retained_scrollbar_drag_finishes_exactly_even_when_release_position_is_unchanged() {
    let mut camera = Camera::default();
    camera.receive(&info(), [1.0; 2]);
    camera.begin_seek();
    assert!(camera.seek(1, 0.300125, 0.3, true));
    let drag = camera.request().unwrap();
    assert!(
        !drag.3,
        "fractional drag frames reuse their current raster phase"
    );
    assert!(camera.end_seek());
    let final_request = camera.request().unwrap();
    assert_eq!(drag.2, final_request.2);
    assert!(final_request.3 && final_request.1 > drag.1);
    assert!(!camera.end_seek());
    assert!(camera.request().is_none());
    camera.begin_seek();
    assert!(camera.seek(1, 0.30025, 0.3, true));
    assert!(!camera.request().unwrap().3);
    camera.cancel();
    assert!(
        camera.request().unwrap().3,
        "focus/pointer cancellation settles the displayed camera"
    );
    assert!(
        !camera.end_seek(),
        "late release after cancellation cannot revive the old drag"
    );
}

#[test]
fn source_map_seek_reverses_and_survives_a_changed_page_density() {
    use midnight_core::{ViewerScrollAnchor, ViewerScrollMap};
    let mut camera = Camera::default();
    let mut frame = info();
    frame.scroll_map = Some(
        ViewerScrollMap::new(
            [(0.0, 0.0), (400.0, 0.1), (650.0, 0.3), (1300.0, 1.0)]
                .into_iter()
                .map(|(y, fraction)| ViewerScrollAnchor { y, fraction })
                .collect(),
        )
        .unwrap(),
    );
    assert!(camera.receive(&frame, [2.25; 2]));
    for y in [525.25, 350.5, 475.75] {
        let target = frame
            .scroll_map
            .as_ref()
            .unwrap()
            .metrics(y, 600.0)
            .unwrap()
            .0;
        assert!(camera.seek(1, target, 0.99, true));
        assert!((camera.origin[1] - y).abs() < 1e-8);
        assert_eq!(camera.origin[0], 100.0);
        // A stale/changed affine page estimate cannot change the shared inverse.
        frame.scroll_per_pixel[1] *= 2.0;
        frame.pixel_id += 1;
        assert!(camera.receive(&frame, [2.25; 2]));
        assert!((camera.origin[1] - y).abs() < 1e-8);
    }
    assert!(
        !camera.seek(1, 1.0, 0.99, false),
        "uncovered endpoint needs the worker"
    );
}

#[test]
fn unchanged_thumb_seek_on_generated_visual_space_preserves_the_camera() {
    use midnight_core::{ViewerScrollAnchor, ViewerScrollMap};
    let mut camera = Camera::default();
    let mut frame = info();
    frame.scroll_map = Some(
        ViewerScrollMap::new(
            [(0.0, 0.2), (1200.0, 0.2), (1400.0, 0.4)]
                .into_iter()
                .map(|(y, fraction)| ViewerScrollAnchor { y, fraction })
                .collect(),
        )
        .unwrap(),
    );
    assert!(camera.receive(&frame, [1.0; 2]));
    assert!(camera.seek(1, f64::from(0.2_f32), 0.2, true));
    assert_eq!(camera.origin, frame.origin);
}
