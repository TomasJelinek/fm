//! Feed native Winit wheel streams to the shared kinetic scroller.
//!
//! Wayland touchpads send phased pixel deltas that end when the fingers lift. X11
//! (XInput2 smooth scrolling) sends unphased fractional line deltas and no lift.
//! Wheel notches keep their eased glide instead: whole lines, and on X11 the
//! 1/120-line steps of high-resolution wheels (libinput's v120 units).
//!
//! The observer delivers only the finger motion that Slint would mishandle: a
//! gesture that began with `Started` (Slint captures it and adds its own fling on
//! the lift) and X11 finger lines (Slint eases each like a notch). Other precise
//! pixels keep Slint's own route.
use crate::kinetic::{KineticScroll, PLATFORM_MOMENTUM};
use i_slint_core::lengths::LogicalPoint;
use slint::winit_030::winit::{
    event::{ElementState, MouseScrollDelta, TouchPhase, WindowEvent},
    keyboard::ModifiersState,
};
use std::time::Duration;

/// Slint's own conversion of one line to logical pixels.
pub(crate) const LINE_PIXELS: f32 = 60.0;
/// A whole-line delta right after fractional ones belongs to the same touchpad stream.
const TOUCHPAD_STREAM: Duration = Duration::from_millis(150);
/// Moving the pointer this far during momentum states a new intent.
const POINTER_SLOP: f32 = 16.0;

/// How the shared tracker classified one native event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelSource {
    /// Not a scroll sample.
    Other,
    /// Finger motion the shared observer delivers itself, without wheel easing.
    Touchpad,
    /// Precise pixels outside a phased gesture: tracked for momentum, but they keep
    /// Slint's own route, where no Slint fling can follow.
    Precise,
    /// A discrete notch; it keeps the eased wheel glide.
    Notch,
}

pub(crate) struct NativeKinetic {
    pub(crate) kinetic: KineticScroll,
    touchpad_until: Option<Duration>,
    /// Logical point of the finger stream; momentum continues on that content.
    pub(crate) origin: Option<LogicalPoint>,
    /// The finger stream arrived as line deltas (X11): hosts convert lines with
    /// their own distance, so momentum is handed to them in lines too.
    pub(crate) lines: bool,
    /// A pixel gesture began with `Started` and has not ended. It follows the
    /// native phases only; stopping momentum does not end it.
    gesture: bool,
    modifiers: ModifiersState,
    /// Synthesise momentum; off where the platform delivers its own.
    pub(crate) enabled: bool,
}

impl Default for NativeKinetic {
    fn default() -> Self {
        Self {
            kinetic: KineticScroll::default(),
            touchpad_until: None,
            origin: None,
            lines: false,
            gesture: false,
            modifiers: ModifiersState::empty(),
            enabled: !PLATFORM_MOMENTUM,
        }
    }
}

/// Finger motion on X11: a fractional line that is not a whole number of
/// high-resolution wheel steps (1/120 line).
fn finger_line(value: f32) -> bool {
    let steps = f64::from(value) * 120.0;
    value.is_finite() && value.fract() != 0.0 && (steps - steps.round()).abs() > 1e-3
}

impl NativeKinetic {
    pub(crate) fn cancel(&mut self) {
        self.kinetic.cancel();
        self.touchpad_until = None;
        self.origin = None;
    }

    /// Ctrl, Alt, Super or Shift change what a wheel event does (zoom, horizontal
    /// scrolling, host shortcuts); none of them may be repeated by momentum.
    pub(crate) fn modifier_held(&self) -> bool {
        !self.modifiers.is_empty()
    }

    /// Ctrl, Alt or Super make a wheel event act (zoom, shortcuts) instead of
    /// scroll; such a notch must reach content once, not as a glide of many events.
    pub(crate) fn wheel_modified(&self) -> bool {
        self.modifiers.control_key() || self.modifiers.alt_key() || self.modifiers.super_key()
    }

    pub(crate) fn reset_modifiers(&mut self) {
        self.modifiers = ModifiersState::empty();
    }

    pub(crate) fn observe(
        &mut self,
        event: &WindowEvent,
        cursor: Option<LogicalPoint>,
        scale: f32,
        now: Duration,
    ) -> WheelSource {
        // Tracked on every platform: content glide must not repeat a modified wheel.
        if let WindowEvent::ModifiersChanged(modifiers) = event {
            if modifiers.state() != self.modifiers {
                self.cancel();
            }
            self.modifiers = modifiers.state();
            return WheelSource::Other;
        }
        if !self.enabled {
            return WheelSource::Other;
        }
        match event {
            WindowEvent::MouseWheel {
                delta: MouseScrollDelta::PixelDelta(delta),
                phase,
                ..
            } => {
                // Winit on Wayland begins the gesture after a wheel notch with
                // `Moved`; Slint captures only gestures that began with `Started`.
                self.gesture |= *phase == TouchPhase::Started;
                let source = if self.gesture {
                    WheelSource::Touchpad
                } else {
                    WheelSource::Precise
                };
                match phase {
                    TouchPhase::Started | TouchPhase::Moved => {
                        if !scale.is_finite() || scale <= 0.0 {
                            return WheelSource::Other;
                        }
                        let delta = delta.to_logical::<f32>(f64::from(scale));
                        self.follow(delta.x, delta.y, cursor, now, false);
                    }
                    TouchPhase::Ended => {
                        self.kinetic.release(now);
                        self.gesture = false;
                    }
                    TouchPhase::Cancelled => {
                        self.cancel();
                        self.gesture = false;
                    }
                }
                return source;
            }
            WindowEvent::MouseWheel {
                delta: MouseScrollDelta::LineDelta(x, y),
                phase: TouchPhase::Moved,
                ..
            } => {
                let stream = self.touchpad_until.is_some_and(|until| now <= until);
                if stream || finger_line(*x) || finger_line(*y) {
                    self.touchpad_until = Some(now + TOUCHPAD_STREAM);
                    self.follow(x * LINE_PIXELS, y * LINE_PIXELS, cursor, now, true);
                    return WheelSource::Touchpad;
                }
                self.cancel();
                return WheelSource::Notch;
            }
            WindowEvent::CursorMoved { .. } => {
                if let (Some(origin), Some(cursor)) = (self.origin, cursor)
                    && self.kinetic.flinging()
                    && (cursor - origin).length() > POINTER_SLOP
                {
                    self.cancel();
                }
            }
            WindowEvent::MouseInput { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::Focused(false)
            | WindowEvent::Destroyed
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::PinchGesture { .. } => self.cancel(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.cancel();
            }
            _ => {}
        }
        WheelSource::Other
    }

    /// Finger motion: tracked for momentum unless a modifier gives the wheel
    /// another meaning; such input still reaches content, once per report.
    fn follow(&mut self, x: f32, y: f32, cursor: Option<LogicalPoint>, now: Duration, lines: bool) {
        if self.modifier_held() {
            self.kinetic.cancel();
            return;
        }
        if self.lines != lines {
            self.kinetic.cancel();
        }
        self.lines = lines;
        self.origin = cursor.or(self.origin);
        self.kinetic.track(x, y, now, !lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::winit_030::winit::{
        dpi::PhysicalPosition,
        event::{DeviceId, MouseButton},
    };

    /// A typical libinput touchpad report on X11: 5.35 px over a 15 px line.
    const FINGER_LINE: f32 = -0.3567;

    fn modifiers(state: ModifiersState) -> WindowEvent {
        WindowEvent::ModifiersChanged(state.into())
    }

    const FRAME: Duration = Duration::from_millis(8);

    fn wheel(delta: MouseScrollDelta, phase: TouchPhase) -> WindowEvent {
        WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta,
            phase,
        }
    }

    fn pixel(y: f64, phase: TouchPhase) -> WindowEvent {
        wheel(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, y)),
            phase,
        )
    }

    fn line(y: f32) -> WindowEvent {
        wheel(MouseScrollDelta::LineDelta(0.0, y), TouchPhase::Moved)
    }

    fn point() -> Option<LogicalPoint> {
        Some(LogicalPoint::new(40.0, 50.0))
    }

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    /// The Linux policy, also on platforms whose own momentum disables it by default.
    fn enabled() -> NativeKinetic {
        NativeKinetic {
            enabled: true,
            ..NativeKinetic::default()
        }
    }

    #[test]
    fn platforms_with_their_own_momentum_get_none_from_us() {
        let mut native = NativeKinetic {
            enabled: false,
            ..NativeKinetic::default()
        };
        let mut now = ms(100);
        for _ in 0..6 {
            assert_eq!(
                native.observe(&pixel(-40.0, TouchPhase::Moved), point(), 1.0, now),
                WheelSource::Other
            );
            now += ms(10);
        }
        native.observe(&pixel(0.0, TouchPhase::Ended), point(), 1.0, now);
        assert!(!native.kinetic.active());
        assert_eq!(NativeKinetic::default().enabled, !PLATFORM_MOMENTUM);
    }

    #[test]
    fn wayland_phased_pixels_fling_after_the_fingers_lift() {
        let mut native = enabled();
        let mut now = ms(100);
        assert_eq!(
            native.observe(&pixel(-40.0, TouchPhase::Started), point(), 2.0, now),
            WheelSource::Touchpad
        );
        for _ in 0..5 {
            now += ms(10);
            native.observe(&pixel(-40.0, TouchPhase::Moved), point(), 2.0, now);
        }
        assert!(!native.kinetic.flinging(), "the finger is still down");
        native.observe(&pixel(0.0, TouchPhase::Ended), point(), 2.0, now);
        assert!(native.kinetic.flinging());
        // 20 logical px per 10 ms upward.
        let (_, y) = native.kinetic.tick(now + FRAME).unwrap();
        assert!(y < -10.0 && y > -20.0, "{y}");
        assert_eq!(native.origin, point());
    }

    #[test]
    fn pixels_outside_a_phased_gesture_keep_the_sdk_route_and_still_coast() {
        let mut native = enabled();
        let mut now = ms(100);
        // A gesture right after a wheel notch arrives without `Started`.
        for _ in 0..6 {
            assert_eq!(
                native.observe(&pixel(-40.0, TouchPhase::Moved), point(), 2.0, now),
                WheelSource::Precise
            );
            now += ms(10);
        }
        assert_eq!(
            native.observe(&pixel(0.0, TouchPhase::Ended), point(), 2.0, now),
            WheelSource::Precise
        );
        assert!(
            native.kinetic.flinging(),
            "the lift starts the shared momentum"
        );
        // A gesture that began with `Started` is the observer's, lift included.
        now += ms(200);
        for phase in [TouchPhase::Started, TouchPhase::Moved, TouchPhase::Ended] {
            assert_eq!(
                native.observe(&pixel(-40.0, phase), point(), 2.0, now),
                WheelSource::Touchpad,
                "{phase:?}"
            );
            now += ms(10);
        }
        assert_eq!(
            native.observe(&pixel(-40.0, TouchPhase::Moved), point(), 2.0, now),
            WheelSource::Precise,
            "the lift ended that gesture"
        );
        // Stopping momentum (a key, a notch) does not end a gesture in progress.
        native.observe(&pixel(-40.0, TouchPhase::Started), point(), 2.0, now);
        native.observe(&line(1.0), point(), 2.0, now + ms(10));
        assert_eq!(
            native.observe(&pixel(-40.0, TouchPhase::Moved), point(), 2.0, now + ms(20)),
            WheelSource::Touchpad
        );
    }

    #[test]
    fn x11_fractional_lines_are_touchpad_and_whole_notches_cancel_momentum() {
        let mut native = enabled();
        let mut now = ms(100);
        for _ in 0..6 {
            assert_eq!(
                native.observe(&line(FINGER_LINE), point(), 1.0, now),
                WheelSource::Touchpad
            );
            now += ms(10);
        }
        assert!(native.lines, "momentum is handed to hosts in lines");
        // An exact whole line inside the stream is still finger motion.
        assert_eq!(
            native.observe(&line(-1.0), point(), 1.0, now),
            WheelSource::Touchpad
        );
        assert!(native.kinetic.tick(now + ms(60)).is_some(), "gap ends it");
        assert!(native.kinetic.flinging());
        // After the stream a notch is a wheel again and replaces the momentum.
        assert_eq!(
            native.observe(&line(1.0), point(), 1.0, now + ms(400)),
            WheelSource::Notch
        );
        assert!(!native.kinetic.active());
    }

    #[test]
    fn press_key_touch_focus_and_pointer_motion_stop_momentum() {
        let interrupts = [
            WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Pressed,
                button: MouseButton::Left,
            },
            // A release stops it as well as a press.
            WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Released,
                button: MouseButton::Middle,
            },
            modifiers(ModifiersState::CONTROL),
            WindowEvent::Focused(false),
            WindowEvent::CursorLeft {
                device_id: DeviceId::dummy(),
            },
            WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(140.0, 50.0),
            },
        ];
        for interrupt in interrupts {
            let mut native = enabled();
            let mut now = ms(100);
            for _ in 0..6 {
                native.observe(&pixel(30.0, TouchPhase::Moved), point(), 1.0, now);
                now += ms(10);
            }
            native.observe(&pixel(0.0, TouchPhase::Ended), point(), 1.0, now);
            assert!(native.kinetic.flinging());
            // A small slip of the pointer while lifting keeps momentum.
            native.observe(
                &WindowEvent::CursorMoved {
                    device_id: DeviceId::dummy(),
                    position: PhysicalPosition::new(45.0, 52.0),
                },
                Some(LogicalPoint::new(45.0, 52.0)),
                1.0,
                now,
            );
            assert!(native.kinetic.flinging());
            let cursor = match interrupt {
                WindowEvent::CursorMoved { position, .. } => {
                    Some(LogicalPoint::new(position.x as f32, position.y as f32))
                }
                _ => point(),
            };
            native.observe(&interrupt, cursor, 1.0, now);
            assert!(!native.kinetic.active(), "{interrupt:?}");
            assert_eq!(native.kinetic.tick(now + FRAME), None);
        }
    }

    #[test]
    fn x11_high_resolution_wheel_steps_are_notches_without_momentum() {
        let mut native = enabled();
        let mut now = ms(100);
        // libinput sends one detent of a high-resolution wheel as 8 x 15/120 line.
        for _ in 0..3 {
            for _ in 0..8 {
                assert_eq!(
                    native.observe(&line(-0.125), point(), 1.0, now),
                    WheelSource::Notch
                );
                now += ms(6);
            }
        }
        for frame in 1..=20 {
            assert_eq!(native.kinetic.tick(now + FRAME * frame), None);
        }
        assert!(!native.kinetic.active());
        assert!(finger_line(FINGER_LINE) && !finger_line(-7.0 / 120.0) && !finger_line(2.0));
    }

    #[test]
    fn held_modifiers_prevent_momentum_and_a_change_cancels_it() {
        let mut native = enabled();
        let mut now = ms(100);
        let flick = |native: &mut NativeKinetic, now: &mut Duration| {
            for index in 0..6 {
                let phase = if index == 0 {
                    TouchPhase::Started
                } else {
                    TouchPhase::Moved
                };
                let source = native.observe(&pixel(-40.0, phase), point(), 2.0, *now);
                assert_eq!(
                    source,
                    WheelSource::Touchpad,
                    "still delivered as finger motion"
                );
                *now += ms(10);
            }
            native.observe(&pixel(0.0, TouchPhase::Ended), point(), 2.0, *now);
        };
        native.observe(&modifiers(ModifiersState::CONTROL), point(), 2.0, now);
        assert!(native.wheel_modified());
        flick(&mut native, &mut now);
        assert!(
            !native.kinetic.active(),
            "Ctrl+flick zooms per report, never after"
        );
        native.observe(&modifiers(ModifiersState::empty()), point(), 2.0, now);
        now += ms(200);
        flick(&mut native, &mut now);
        assert!(native.kinetic.flinging());
        native.observe(&modifiers(ModifiersState::SHIFT), point(), 2.0, now);
        assert!(!native.kinetic.active(), "a modifier change stops momentum");
        assert!(!native.wheel_modified() && native.modifier_held());
    }
}
