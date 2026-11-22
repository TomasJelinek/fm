//! Touchpad momentum through the real panels: tracking, decay, cancellation, edges.
use super::*;
use i_slint_core::input::{BackendMouseEvent, TouchPhase as SlintPhase};
use slint::winit_030::winit::event::MouseButton;

const SAMPLE: Duration = Duration::from_millis(10);
/// A typical libinput touchpad report on X11: 5.35 px over a 15 px line.
const FINGER_LINE: f32 = -0.3567;

fn wait(h: &Harness, time: Duration) {
    h.time.set(h.time.get() + time);
    slint::platform::update_timers_and_animations();
}

fn pointer_at(h: &Harness, x: f32, y: f32) -> slint::LogicalPosition {
    h.point(x, y);
    slint::LogicalPosition::new(x, y)
}

fn pointer_at_left_list(h: &Harness) -> slint::LogicalPosition {
    pointer_at(
        h,
        h.window.get_left_list_left() + 100.0,
        h.window.get_left_list_top() + 100.0,
    )
}

/// One Wayland touchpad report. Input the hook leaves alone reaches Slint the way
/// the Winit adapter delivers it: a wheel with its phase.
fn finger(h: &Harness, at: slint::LogicalPosition, y: f32, phase: TouchPhase) {
    wait(h, SAMPLE);
    if !h.input(WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, y.into())),
        phase,
    }) {
        h.window
            .window()
            .dispatch_event(slint::platform::WindowEvent::internal(
                BackendMouseEvent::Wheel {
                    position: LogicalPoint::new(at.x, at.y),
                    delta_x: 0.0,
                    delta_y: y,
                    phase: match phase {
                        TouchPhase::Started => SlintPhase::Started,
                        TouchPhase::Moved => SlintPhase::Moved,
                        TouchPhase::Ended => SlintPhase::Ended,
                        TouchPhase::Cancelled => SlintPhase::Cancelled,
                    },
                },
            ));
    }
}

fn flick(h: &Harness, at: slint::LogicalPosition, y: f32) {
    finger(h, at, y, TouchPhase::Started);
    for _ in 0..5 {
        finger(h, at, y, TouchPhase::Moved);
    }
    finger(h, at, 0.0, TouchPhase::Ended);
}

fn kinetic(h: &Harness) -> bool {
    h.auxiliary.borrow().kinetic_active()
}

#[test]
fn phased_touchpad_flick_coasts_once_with_decaying_steps_then_settles() {
    let h = layout_harness();
    let at = pointer_at_left_list(&h);
    flick(&h, at, -20.0);
    let released = h.offset(0);
    assert!(
        (released + 120.0).abs() < 0.01,
        "finger motion is applied as it arrives: {released}"
    );
    assert!(kinetic(&h));
    let mut previous = released;
    let mut previous_step = f32::INFINITY;
    for frame in 0..20 {
        h.advance(1);
        let current = h.offset(0);
        let step = previous - current;
        assert!(step > 0.0, "frame {frame}: momentum continues the flick");
        // 2000 px/s for one 16 ms frame; a second fling (Slint's own Flickable fling
        // on the lift) on top of the shared one would show as a larger step.
        assert!(
            step < 33.0,
            "frame {frame}: no jump or double fling ({step} px)"
        );
        assert!(
            step <= previous_step + 0.01,
            "frame {frame}: friction slows it"
        );
        assert!(h.window.get_scroll_interaction_active());
        assert_eq!(h.offset(1), 0.0, "the other panel stays still");
        previous = current;
        previous_step = step;
    }
    h.settle(400);
    assert!(!kinetic(&h));
    let travel = released - h.offset(0);
    // 2000 px/s with a 0.8 s time constant: about 1590 px (the dropped crawl below
    // 12 px/s is 9.6 px); idle row alignment may add a row.
    assert!(
        travel > 1530.0 && travel < 1630.0,
        "momentum travel {travel}"
    );
}

#[test]
fn x11_fractional_line_stream_follows_the_finger_then_coasts() {
    let h = layout_harness();
    pointer_at_left_list(&h);
    for _ in 0..6 {
        wait(&h, SAMPLE);
        assert!(h.wheel(FINGER_LINE), "panel delivers touchpad lines itself");
    }
    h.advance(1);
    let finger = 6.0 * FINGER_LINE * LINE_PIXELS;
    assert!(
        (h.offset(0) - finger).abs() < 0.05,
        "finger motion arrives without wheel easing: {}",
        h.offset(0)
    );
    assert!(kinetic(&h), "waiting for the end of the unphased stream");
    h.settle(400);
    let travel = f64::from(finger - h.offset(0));
    // The finger's speed (X11 lines at the shared line distance) over the shared
    // time constant, less the dropped crawl below 12 px/s.
    let speed = f64::from(FINGER_LINE.abs() * LINE_PIXELS) / 0.010;
    let expected = (speed - 12.0) * TIME_CONSTANT;
    assert!(
        (travel - expected).abs() < 60.0,
        "no lift event, yet momentum follows: {travel} of about {expected}"
    );
}

#[test]
fn x11_high_resolution_wheel_detent_glides_one_notch_without_momentum() {
    let h = layout_harness();
    pointer_at_left_list(&h);
    // libinput reports one detent of a high-resolution wheel as 8 x 15/120 line.
    for _ in 0..8 {
        wait(&h, Duration::from_millis(6));
        assert!(h.wheel(-0.125));
    }
    assert!(!kinetic(&h), "wheel steps are no touchpad stream");
    h.settle(400);
    assert!(
        (h.offset(0) + line_pixels()).abs() < 22.0,
        "one detent moves one notch (plus idle row alignment): {}",
        h.offset(0)
    );
}

#[test]
fn a_click_stops_momentum_at_once() {
    let h = layout_harness();
    let at = pointer_at_left_list(&h);
    flick(&h, at, -20.0);
    h.advance(3);
    for state in [ElementState::Pressed, ElementState::Released] {
        h.input(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        });
    }
    assert!(!kinetic(&h));
    let stopped = h.offset(0);
    h.settle(100);
    assert!(
        (h.offset(0) - stopped).abs() < 22.0,
        "only idle row alignment may follow: {stopped} -> {}",
        h.offset(0)
    );
}

#[test]
fn momentum_against_the_top_stops_without_overshoot() {
    let h = layout_harness();
    let at = pointer_at_left_list(&h);
    flick(&h, at, 20.0);
    assert_eq!(h.offset(0), 0.0);
    assert!(kinetic(&h));
    for _ in 0..12 {
        h.advance(1);
        assert_eq!(h.offset(0), 0.0, "content never passes its start");
    }
    assert!(!kinetic(&h), "the stalled edge ends momentum early");
    assert!(!h.window.get_scroll_interaction_active());
}

#[test]
fn a_flick_outside_scrollable_lists_has_no_momentum() {
    let h = layout_harness();
    // The toolbar row above the panels: no list scrolls there.
    let at = pointer_at(&h, h.window.get_left_list_left() + 100.0, 4.0);
    flick(&h, at, -20.0);
    h.advance(1);
    assert!(!kinetic(&h), "momentum needs content that scrolls along it");
    assert!(!h.window.get_scroll_interaction_active());
    assert_eq!((h.offset(0), h.offset(1)), (0.0, 0.0));
}

/// The shared momentum's decay time constant (`midnight-ui-input` `kinetic.rs`
/// `TIME_CONSTANT`). The viewers' tests pin the same value: change them together.
const TIME_CONSTANT: f64 = 0.8;

/// The shared momentum model, as the text viewer's kinetic tests see it: the
/// distance an exponential decay from `velocity` (px/s) covers between `from` and
/// `to` seconds after the lift.
fn momentum(velocity: f64, from: f64, to: f64) -> f64 {
    velocity * TIME_CONSTANT * ((-from / TIME_CONSTANT).exp() - (-to / TIME_CONSTANT).exp())
}

/// One presented frame `millis` after the previous one, as native redraw paces them.
fn frame(h: &Harness, millis: u64) -> f32 {
    h.time.set(h.time.get() + Duration::from_millis(millis));
    slint::platform::update_timers_and_animations();
    h.input(WindowEvent::RedrawRequested);
    h.draw();
    h.offset(0)
}

fn on_row(h: &Harness, offset: f32) -> bool {
    let row = h.window.get_left_view_row_height();
    (offset / row - (offset / row).round()).abs() < 1e-3
}

/// The text viewer's kinetic test flick (six 20 px reports 10 ms apart), frame for
/// frame in a panel: the same velocity and the same 8 ms steps of the same decay.
/// The row alignment begins only after the momentum has ended and the list has
/// rested for the idle interval.
#[test]
fn a_touchpad_flick_coasts_exactly_like_the_text_viewers_then_settles_on_a_row() {
    let h = layout_harness();
    let at = pointer_at_left_list(&h);
    flick(&h, at, -20.0);
    let released = h.offset(0);
    assert!((released + 120.0).abs() < 0.01, "finger motion: {released}");
    let mut previous = released;
    let mut frames = 0;
    while kinetic(&h) {
        let current = frame(&h, 8);
        let elapsed = f64::from(frames) * 0.008;
        let expected = momentum(2000.0, elapsed, elapsed + 0.008) as f32;
        let step = previous - current;
        if kinetic(&h) {
            assert!(
                (step - expected).abs() < 2e-3,
                "frame {frames}: {step} px; the shared model moves {expected} px"
            );
            assert!(h.window.get_scroll_interaction_active());
        }
        if frames == 29 {
            // As in the viewer's test: 240 ms after the lift an 8 ms frame still
            // moves about 12 of the 16 px it started with.
            assert!(step > 9.0, "the glide keeps going: {step}");
        }
        previous = current;
        frames += 1;
        assert!(frames < 700, "momentum ends below 12 px/s");
    }
    // 2000 px/s over the time constant, less the dropped crawl below 12 px/s.
    let coasted = f64::from(released - previous);
    let travel = (2000.0 - 12.0) * TIME_CONSTANT;
    assert!(
        (coasted - travel).abs() < 2.0 + 12.0 * TIME_CONSTANT,
        "travel of the shared model: {coasted} of about {travel}"
    );
    // The list rests through the idle interval, then glides to the nearest row.
    let rest = previous;
    let mut rested = 0;
    while frame(&h, 8) == rest {
        rested += 1;
        assert!(rested < 40, "the alignment follows the rest");
    }
    assert!(
        rested >= 11,
        "rows settle only after an idle interval: {rested} frames of 8 ms"
    );
    h.settle(100);
    let row = h.window.get_left_view_row_height();
    assert!(on_row(&h, h.offset(0)), "settled on a row: {}", h.offset(0));
    assert!(
        (h.offset(0) - rest).abs() <= row / 2.0 + 0.01,
        "the nearest row: {rest} -> {}",
        h.offset(0)
    );
}

/// Windows and macOS deliver their own momentum as further wheel events after the
/// fingers lift; a Windows precision touchpad's are fractional lines. The panel
/// glides them exactly as the text viewer does: their share of the notch distance,
/// each eased by the shared wheel glide, only undelivered distance adding up, and
/// no row quantization or alignment before the stream and its glide have ended.
#[test]
fn a_windows_touchpad_stream_glides_frame_for_frame_like_the_text_viewer() {
    let h = harness_entries(false, false, || {
        (0..1000)
            .map(|id| {
                Entry::new(
                    EntryId(id),
                    PathBuf::from(format!("file-{id:04}.txt")),
                    EntryKind::File,
                )
            })
            .collect()
    });
    pointer_at_left_list(&h);
    // The text viewer's model of wheel lines (image_motion pushes the notch distance
    // per line into a WheelMotion, ticked on presented frames).
    let mut viewer = WheelMotion::default();
    let start = h.offset(0);
    let mut expected = start;
    // Twelve finger reports, then the system's decaying momentum, 8 ms apart.
    let reports: Vec<f32> = (0..12)
        .map(|_| -0.3)
        .chain((0..40).map(|k| -0.3 * 0.92f32.powi(k)))
        .collect();
    let mut total = 0.0;
    for (index, lines) in reports.iter().enumerate() {
        h.time.set(h.time.get() + Duration::from_millis(8));
        let now = h.time.get();
        assert!(h.wheel(*lines), "report {index} is the panel's");
        viewer.push(0.0, lines * line_pixels(), now);
        total += lines * line_pixels();
        let current = frame(&h, 0);
        expected += viewer.tick(now).map_or(0.0, |(_, y)| y);
        assert!(
            (current - expected).abs() < 0.01,
            "report {index}: panel {current}, viewer model {expected}"
        );
        assert!(h.window.get_scroll_interaction_active());
    }
    // The glide's tail after the last report, frame for frame.
    while viewer.active() {
        let current = frame(&h, 8);
        expected += viewer.tick(h.time.get()).map_or(0.0, |(_, y)| y);
        assert!((current - expected).abs() < 0.01, "{current} vs {expected}");
    }
    assert!(
        (h.offset(0) - start - total).abs() < 0.01,
        "the whole stream's distance: {} of {total}",
        h.offset(0) - start
    );
    let rest = h.offset(0);
    h.settle(100);
    assert!(
        on_row(&h, h.offset(0)),
        "then the rows settle: {}",
        h.offset(0)
    );
    assert!((h.offset(0) - rest).abs() <= 11.01);
}

/// A notch eases on the text viewer's curve (the same share of its distance on
/// every frame, the same accumulation and reversal). Its distance is the whole
/// rows nearest to the viewer's notch, so it lands on a row and no alignment
/// follows it.
#[test]
fn wheel_notches_glide_on_the_viewers_curve_and_land_on_rows() {
    let h = layout_harness();
    pointer_at_left_list(&h);
    let row = h.window.get_left_view_row_height();
    assert_eq!(row, 22.0);
    let notch = notch(row);
    let mut viewer = WheelMotion::default();
    let start = h.time.get();
    assert!(h.wheel(-1.0));
    viewer.push(0.0, -line_pixels(), start);
    let mut previous = h.offset(0);
    let mut delivered = 0.0;
    while viewer.active() {
        let current = frame(&h, 16);
        let (_, step) = viewer.tick(h.time.get()).unwrap_or_default();
        delivered += step;
        assert!(
            ((current - previous) / notch - step / line_pixels()).abs() < 1e-4,
            "panel {} of {notch} px, viewer {step} of {} px",
            current - previous,
            line_pixels()
        );
        previous = current;
    }
    assert!((delivered + line_pixels()).abs() < 1e-3);
    assert!((h.offset(0) + notch).abs() < 1e-3, "rows: {}", h.offset(0));
    for _ in 0..40 {
        assert!(
            (frame(&h, 16) + notch).abs() < 1e-3,
            "no alignment after a notch"
        );
    }
    // Notches add up while gliding.
    assert!(h.wheel(-1.0));
    frame(&h, 16);
    assert!(h.wheel(-1.0));
    h.settle(60);
    assert!(
        (h.offset(0) + 3.0 * notch).abs() < 1e-3,
        "three notches: {}",
        h.offset(0)
    );
    // A reversal drops the old remainder and lands on a row above.
    assert!(h.wheel(-3.0));
    frame(&h, 16);
    let turned = frame(&h, 16);
    assert!(h.wheel(1.0));
    h.settle(60);
    let back = h.offset(0);
    assert!(on_row(&h, back), "a reversal lands on a row: {back}");
    assert!(
        back > turned && back - turned <= notch + row / 2.0,
        "{turned} -> {back}"
    );
}

/// The thumbnail grid: a notch moves one band and lands on its boundary, while
/// touchpad lines move the viewer's line distance and keep that pixel position:
/// rounding its tall bands would undo a small gesture.
#[test]
fn the_thumbnail_grid_moves_a_band_per_notch_and_keeps_touchpad_positions() {
    let h = harness_entries(false, false, || {
        (0..1000)
            .map(|id| {
                Entry::new(
                    EntryId(id),
                    PathBuf::from(format!("file-{id:04}.txt")),
                    EntryKind::File,
                )
            })
            .collect()
    });
    h.window.invoke_view_mode(0, 2);
    h.advance(1);
    pointer_at_left_list(&h);
    let band = h.window.get_left_view_row_height();
    assert!(band > 120.0, "a thumbnail band: {band}");
    assert!(h.wheel(-1.0));
    h.settle(60);
    assert!(
        (h.offset(0) + band).abs() < 1e-3,
        "one band: {}",
        h.offset(0)
    );
    for _ in 0..4 {
        h.time.set(h.time.get() + Duration::from_millis(8));
        assert!(h.wheel(-0.1));
    }
    h.settle(100);
    assert!(
        (h.offset(0) + band + 0.4 * line_pixels()).abs() < 0.01,
        "touchpad pixels stay: {}",
        h.offset(0)
    );
}

#[test]
fn a_modifier_a_click_or_a_notch_stops_momentum_and_only_alignment_follows() {
    // One window per test thread: each case starts again at the top.
    let h = layout_harness();
    let at = pointer_at_left_list(&h);
    for interrupt in 0..3 {
        h.window.set_left_cursor(0);
        h.window.invoke_reveal_left();
        h.settle(100);
        assert_eq!(h.offset(0), 0.0);
        flick(&h, at, -20.0);
        h.advance(3);
        assert!(kinetic(&h));
        match interrupt {
            // Winit's key events cannot be built outside Winit; a modifier key's
            // change takes the same cancelling path in the shared tracker.
            0 => {
                h.input(WindowEvent::ModifiersChanged(
                    slint::winit_030::winit::keyboard::ModifiersState::SHIFT.into(),
                ));
                h.input(WindowEvent::ModifiersChanged(
                    slint::winit_030::winit::keyboard::ModifiersState::empty().into(),
                ));
            }
            1 => {
                for state in [ElementState::Pressed, ElementState::Released] {
                    h.input(WindowEvent::MouseInput {
                        device_id: DeviceId::dummy(),
                        state,
                        button: MouseButton::Left,
                    });
                }
            }
            _ => {
                assert!(h.wheel(-1.0));
            }
        }
        assert!(!kinetic(&h), "interrupt {interrupt} stops the momentum");
        let stopped = h.offset(0);
        h.settle(100);
        let moved = (h.offset(0) - stopped).abs();
        let allowed = if interrupt == 2 {
            notch(22.0) + 11.0
        } else {
            11.0
        };
        assert!(
            moved <= allowed + 0.01 && on_row(&h, h.offset(0)),
            "interrupt {interrupt}: {stopped} -> {}",
            h.offset(0)
        );
    }
}

/// With the system's animation effects off, a notch moves its rows at once.
#[test]
fn reduced_motion_moves_a_notch_at_once() {
    let h = layout_harness();
    h.state.borrow_mut().reduce_motion = true;
    pointer_at_left_list(&h);
    assert!(h.wheel(-1.0));
    assert!(
        (frame(&h, 16) + notch(22.0)).abs() < 1e-3,
        "{}",
        h.offset(0)
    );
}

/// Fingers resting on a Wayland touchpad hold the list where they left it: the
/// alignment waits for the lift (a held phased gesture is scroll interaction).
#[test]
fn resting_touchpad_fingers_hold_the_alignment_until_they_lift() {
    let h = layout_harness();
    let at = pointer_at_left_list(&h);
    finger(&h, at, -7.0, TouchPhase::Started);
    finger(&h, at, -7.0, TouchPhase::Moved);
    let held = h.offset(0);
    assert!((held + 14.0).abs() < 0.01, "finger motion: {held}");
    for _ in 0..60 {
        assert_eq!(frame(&h, 16), held, "the resting fingers hold the list");
    }
    // A slow lift: no momentum, then the rows settle.
    wait(&h, Duration::from_millis(200));
    finger(&h, at, 0.0, TouchPhase::Ended);
    assert!(!kinetic(&h));
    h.settle(100);
    assert!(on_row(&h, h.offset(0)), "{}", h.offset(0));
    assert!((h.offset(0) - held).abs() <= 11.01);
}
