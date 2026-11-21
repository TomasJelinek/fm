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
    let finger = 6.0 * FINGER_LINE * 60.0;
    assert!(
        (h.offset(0) - finger).abs() < 0.05,
        "finger motion arrives without wheel easing: {}",
        h.offset(0)
    );
    assert!(kinetic(&h), "waiting for the end of the unphased stream");
    h.settle(400);
    let travel = finger - h.offset(0);
    // 21.4 px every 10 ms is 2140 px/s: about 1702 px with the 0.8 s time constant.
    assert!(
        travel > 1650.0 && travel < 1760.0,
        "no lift event, yet momentum follows: {travel}"
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
        (h.offset(0) + 60.0).abs() < 22.0,
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
