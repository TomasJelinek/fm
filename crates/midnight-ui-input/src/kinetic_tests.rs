use super::*;

const FRAME: Duration = Duration::from_millis(8);

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

/// A finger stroke: one report every 10 ms with these vertical steps, ending at `end`.
fn steps(kinetic: &mut KineticScroll, steps: &[f32], end: Duration, phased: bool) {
    let count = steps.len() as u64;
    for (index, &step) in steps.iter().enumerate() {
        kinetic.track(0.0, step, end - ms(10 * (count - 1 - index as u64)), phased);
    }
}

/// A steady finger: `count` samples of `step` pixels every 10 ms ending at `end`.
fn stroke(kinetic: &mut KineticScroll, step: f32, count: usize, end: Duration, phased: bool) {
    steps(kinetic, &vec![step; count], end, phased);
}

/// Run the fling to completion and return the total delivered vertical travel.
fn coast(kinetic: &mut KineticScroll, from: Duration) -> f64 {
    let mut total = 0.0;
    for frame in 1..=2000 {
        if let Some((_, y)) = kinetic.tick(from + FRAME * frame) {
            total += f64::from(y);
        } else if !kinetic.active() {
            break;
        }
    }
    assert!(!kinetic.active(), "momentum must finish");
    total
}

#[test]
fn velocity_is_the_newest_finger_speed_and_a_rest_is_zero() {
    let mut kinetic = KineticScroll::default();
    // Fast at first, then 10 px every 10 ms for the last 50 ms before the lift.
    steps(
        &mut kinetic,
        &[40.0, 40.0, 40.0, 40.0, 40.0, 10.0, 10.0, 10.0, 10.0, 10.0],
        ms(190),
        true,
    );
    let velocity = kinetic.velocity(ms(190));
    assert_eq!(velocity[0], 0.0);
    assert!((velocity[1] - 1000.0).abs() < 1.0, "{velocity:?}");
    // A finger that rested more than 40 ms before lifting has stopped.
    assert_eq!(kinetic.velocity(ms(190) + MAX_REST + ms(1)), [0.0; 2]);
    assert!(!kinetic.release(ms(190) + ms(60)));
}

#[test]
fn unknown_speeds_never_fling() {
    let mut kinetic = KineticScroll::default();
    kinetic.track(0.0, -400.0, ms(10), true);
    kinetic.track(0.0, -400.0, ms(20), true);
    assert_eq!(
        kinetic.velocity(ms(20)),
        [0.0; 2],
        "two samples are no flick"
    );
    // Reports queued during a UI stall arrive together with one timestamp.
    for _ in 0..8 {
        kinetic.track(30.0, -40.0, ms(500), true);
    }
    assert_eq!(kinetic.velocity(ms(500)), [0.0; 2]);
    assert!(!kinetic.release(ms(500)), "a burst is no measured flick");
    assert!(!kinetic.active());
}

#[test]
fn reversal_discards_the_previous_direction() {
    let mut kinetic = KineticScroll::default();
    stroke(&mut kinetic, 20.0, 5, ms(100), true);
    stroke(&mut kinetic, -10.0, 4, ms(140), true);
    let velocity = kinetic.velocity(ms(140));
    assert!((velocity[1] + 1000.0).abs() < 1.0, "{velocity:?}");
}

#[test]
fn release_flings_and_decays_exponentially_to_a_stop() {
    let mut kinetic = KineticScroll::default();
    stroke(&mut kinetic, 20.0, 6, ms(100), true);
    assert!(kinetic.release(ms(105)));
    assert!(kinetic.flinging());
    let mut deltas = Vec::new();
    for frame in 1..=40 {
        if let Some((x, y)) = kinetic.tick(ms(105) + FRAME * frame) {
            assert_eq!(x, 0.0);
            deltas.push(y);
        }
    }
    assert_eq!(deltas.len(), 40);
    // First frame continues at about the finger speed: 2000 px/s * 8 ms = 16 px.
    assert!((deltas[0] - 16.0).abs() < 1.0, "{}", deltas[0]);
    // Each frame is smaller than the previous by the same factor (friction).
    let ratio = f64::from(deltas[1] / deltas[0]);
    for pair in deltas.windows(2) {
        assert!(pair[1] < pair[0]);
        assert!((f64::from(pair[1] / pair[0]) - ratio).abs() < 1e-3);
    }
    let rest = coast(&mut kinetic, ms(105) + FRAME * 40);
    let total = deltas.iter().map(|&step| f64::from(step)).sum::<f64>() + rest;
    // Total travel is velocity * time constant (0.8 s): a 2000 px/s flick glides
    // about 1600 px, less the invisible crawl dropped below 12 px/s (9.6 px).
    assert!(total < 1600.0 && total > 1589.0, "{total}");
}

#[test]
fn slow_frames_keep_the_glide_in_real_time() {
    // A missed vsync or a slow (debug) frame must not slow the glide below the
    // finger's speed: after the same time, every frame rate has moved as far.
    let travel_after_100_ms = |frame: Duration| {
        let mut kinetic = KineticScroll::default();
        stroke(&mut kinetic, 20.0, 6, ms(100), true);
        assert!(kinetic.release(ms(100)));
        let mut total = 0.0;
        let mut now = ms(100);
        while now < ms(200) {
            now += frame;
            total += kinetic.tick(now).map_or(0.0, |(_, y)| f64::from(y));
        }
        total
    };
    let smooth = travel_after_100_ms(ms(10));
    for frame in [ms(25), ms(50)] {
        let slow = travel_after_100_ms(frame);
        assert!(
            (slow - smooth).abs() < 0.01,
            "{frame:?} frames: {slow} vs {smooth}"
        );
    }
}

#[test]
fn travel_does_not_depend_on_the_frame_rate() {
    let travel = |frame: Duration| {
        let mut kinetic = KineticScroll::default();
        stroke(&mut kinetic, -15.0, 8, ms(100), true);
        kinetic.release(ms(100));
        let mut total = 0.0;
        let mut now = ms(100);
        while kinetic.active() {
            now += frame;
            total += kinetic.tick(now).map_or(0.0, |(_, y)| f64::from(y));
        }
        total
    };
    let at_60 = travel(ms(16));
    let at_120 = travel(FRAME);
    assert!(at_60 < 0.0);
    assert!((at_60 - at_120).abs() < 2.0, "{at_60} vs {at_120}");
}

#[test]
fn slow_release_does_not_fling() {
    let mut kinetic = KineticScroll::default();
    stroke(&mut kinetic, 2.0, 6, ms(100), true);
    assert!(
        !kinetic.release(ms(100)),
        "200 px/s is a placement, not a flick"
    );
    assert!(!kinetic.active());
    assert_eq!(kinetic.tick(ms(200)), None);
}

#[test]
fn delayed_frames_do_not_jump() {
    let mut kinetic = KineticScroll::default();
    stroke(&mut kinetic, 40.0, 6, ms(100), true);
    kinetic.release(ms(100));
    let (_, step) = kinetic.tick(ms(100) + Duration::from_secs(2)).unwrap();
    // 4000 px/s over one capped 50 ms frame at most, not two seconds of travel.
    assert!(step > 0.0 && step <= 200.0, "{step}");
}

#[test]
fn new_touch_and_cancel_stop_momentum_immediately() {
    let mut kinetic = KineticScroll::default();
    stroke(&mut kinetic, 20.0, 6, ms(100), true);
    kinetic.release(ms(100));
    assert!(kinetic.tick(ms(108)).is_some());
    kinetic.track(0.0, 0.5, ms(110), true);
    assert!(!kinetic.flinging(), "a new finger takes over");
    assert_eq!(kinetic.tick(ms(118)), None);
    stroke(&mut kinetic, 20.0, 6, ms(300), true);
    kinetic.release(ms(300));
    kinetic.cancel();
    assert!(!kinetic.active());
    assert_eq!(kinetic.tick(ms(308)), None);
}

#[test]
fn unphased_streams_end_after_a_short_gap_and_continue_without_a_jump() {
    let mut kinetic = KineticScroll::default();
    stroke(&mut kinetic, 20.0, 6, ms(100), false);
    assert!(
        kinetic.active(),
        "waiting for the end of an unphased stream"
    );
    assert!(!kinetic.flinging());
    // Within the gap the finger may still be down: nothing is synthesised.
    assert_eq!(kinetic.tick(ms(110)), None);
    // 10 ms intervals: the stream ends after 30 ms.
    let (_, first) = kinetic.tick(ms(130)).unwrap();
    assert!(kinetic.flinging());
    assert!(first > 0.0 && first <= 2000.0 * 0.016 + 0.5, "{first}");
    // A late sample means the finger was still down: momentum yields to it.
    kinetic.track(0.0, 20.0, ms(132), false);
    assert!(!kinetic.flinging());
    // A late frame notices the end later; the gap is still not caught up at once.
    let mut late = KineticScroll::default();
    stroke(&mut late, 20.0, 6, ms(100), false);
    let (_, first) = late.tick(ms(150)).unwrap();
    assert!(first > 0.0 && first <= 2000.0 * 0.016 + 0.5, "{first}");
}

#[test]
fn unphased_streams_coast_only_after_a_clear_flick() {
    // Without a lift event a deliberate stop looks like a lift: it must not coast.
    let ended = |strokes: &[f32]| {
        let mut kinetic = KineticScroll::default();
        steps(&mut kinetic, strokes, ms(200), false);
        for frame in 1..=20 {
            kinetic.tick(ms(200) + FRAME * frame);
        }
        kinetic.flinging()
    };
    assert!(!ended(&[6.0; 8]), "600 px/s then a stop");
    assert!(
        !ended(&[30.0, 25.0, 20.0, 15.0, 10.0, 5.0]),
        "a stroke slowing to a stop"
    );
    assert!(ended(&[10.0; 8]), "a steady 1000 px/s flick");
    // A phased lift at the slower speed does fling: the lift itself is reported.
    let mut kinetic = KineticScroll::default();
    stroke(&mut kinetic, 6.0, 8, ms(200), true);
    assert!(kinetic.release(ms(205)));
}

#[test]
fn stalled_content_stops_only_that_axis() {
    let mut kinetic = KineticScroll::default();
    for index in 0..6 {
        kinetic.track(15.0, -20.0, ms(50 + 10 * index), true);
    }
    kinetic.release(ms(100));
    let mut now = ms(100);
    let mut x = 0.0;
    // Vertical content sits at its end: the position never changes.
    while now < ms(100) + STALL_TIMEOUT + FRAME * 2 {
        now += FRAME;
        let (dx, _) = kinetic.tick(now).unwrap();
        x += f64::from(dx);
        kinetic.observe_position([Some(x), Some(-500.0)], now);
    }
    let (dx, dy) = kinetic.tick(now + FRAME).unwrap();
    assert!(dx > 0.0);
    assert_eq!(dy, 0.0, "the vertical edge stopped vertical momentum");
    assert_eq!(
        kinetic.fling_velocity().map(|velocity| velocity[1]),
        Some(0.0)
    );
    kinetic.stop_axis(0);
    assert!(!kinetic.active());
}

#[test]
fn invalid_input_is_ignored() {
    let mut kinetic = KineticScroll::default();
    kinetic.track(f32::NAN, 1.0, ms(0), true);
    kinetic.track(0.0, f32::INFINITY, ms(1), true);
    assert_eq!(kinetic.velocity(ms(1)), [0.0; 2]);
    assert!(!kinetic.release(ms(2)));
    kinetic.observe_position([Some(f64::NAN), None], ms(3));
    kinetic.stop_axis(7);
    assert!(!kinetic.active());
}

#[test]
fn the_glide_scale_lengthens_momentum_and_zero_starts_none() {
    // The scale is this thread's; it is restored before the assertions.
    let travel = |scale: f32| {
        crate::set_glide_scale(scale);
        let mut kinetic = KineticScroll::default();
        stroke(&mut kinetic, -20.0, 6, ms(100), true);
        let flung = kinetic.release(ms(100));
        crate::set_glide_scale(1.0);
        (
            flung,
            if flung {
                coast(&mut kinetic, ms(100))
            } else {
                0.0
            },
        )
    };
    let (flung, normal) = travel(1.0);
    assert!(flung);
    let (_, doubled) = travel(2.0);
    // 2000 px/s decays over 0.8 s, and over 1.6 s at twice the scale: twice the travel,
    // less the crawl dropped at the end.
    assert!(
        (doubled / normal - 2.0).abs() < 0.02,
        "{normal} and {doubled}"
    );
    assert_eq!(travel(0.0), (false, 0.0));
}

#[test]
fn a_flick_coasts_on_top_of_the_finger_at_every_glide_stop() {
    // The hosts deliver the finger's own motion directly at every stop; the momentum only
    // adds to it, in the flick's direction, and grows with the stop. Off adds none.
    let coasts: Vec<f64> = [0.0, 0.25, 0.5, 1.0, 2.0, 4.0]
        .into_iter()
        .map(|scale| {
            crate::set_glide_scale(scale);
            let mut kinetic = KineticScroll::default();
            stroke(&mut kinetic, -20.0, 6, ms(100), true);
            let flung = kinetic.release(ms(100));
            crate::set_glide_scale(1.0);
            // Very long coasts longer than `coast` runs.
            let mut total = 0.0;
            for frame in 1..=4000 {
                if let Some((_, y)) = kinetic.tick(ms(100) + FRAME * frame) {
                    total += f64::from(y);
                }
            }
            assert_eq!(flung, scale > 0.0);
            assert!(!kinetic.active(), "momentum must finish at x{scale}");
            total
        })
        .collect();
    assert_eq!(coasts[0], 0.0);
    for pair in coasts.windows(2) {
        assert!(pair[1] <= pair[0] && pair[1] < 0.0, "{coasts:?}");
    }
}
