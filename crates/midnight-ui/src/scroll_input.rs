//! Observe native interaction without consuming row clicks or touch gestures.
#[path = "wheel_motion.rs"]
mod wheel_motion;

use crate::MainWindow;
use i_slint_core::{
    lengths::{LogicalPoint, LogicalRect, LogicalSize},
    window::{WindowInner, WindowKind},
};
use midnight_ui_input::{
    CommandStrip, LINE_PIXELS, NativeScroll, PressState, WheelSetting, notch_pixels_for,
};
#[cfg(test)]
use slint::winit_030::winit::event::DeviceId;
use slint::{
    ComponentHandle, Model,
    winit_030::{
        EventResult, WinitWindowAccessor,
        winit::{
            dpi::PhysicalPosition,
            event::{ElementState, MouseScrollDelta, TouchPhase, WindowEvent},
        },
    },
};
use std::{cell::RefCell, rc::Rc, time::Duration};
use wheel_motion::{WHEEL_FRAME, WheelMotion};

#[derive(Default)]
struct InputState {
    pressed: PressState,
    cursor: Option<PhysicalPosition<f64>>,
    wheel_position: Option<slint::LogicalPosition>,
    wheel: WheelMotion,
    target: Option<WheelTarget>,
    /// Row (band) height of the list under the pointer; 0 where unknown.
    line_height: f32,
    /// That list's content-y: a whole notch lands on one of its row boundaries.
    content_y: f32,
    /// That list's visible size: one screen per notch moves it.
    page: [f32; 2],
    /// The system asks for less motion: wheel input moves the list at once.
    reduce_motion: bool,
    /// The system's wheel setting (lines or one screen per notch).
    wheel_setting: WheelSetting,
}

#[derive(PartialEq)]
struct WheelTarget {
    pane: usize,
    mode: i32,
    path: slint::SharedString,
    tab: slint::SharedString,
}

fn wheel_target(window: &MainWindow, position: slint::LogicalPosition) -> Option<WheelTarget> {
    // Popup menus and dialogs own their normal native wheel hit-testing. This
    // pinned internal API is already used by our input-transparent tooltips.
    if WindowInner::from_pub(window.window())
        .active_popups()
        .iter()
        .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
    {
        return None;
    }
    let bounds = [
        (
            window.get_left_list_left(),
            window.get_left_list_top(),
            window.get_left_list_width(),
            window.get_left_list_height(),
        ),
        (
            window.get_right_list_left(),
            window.get_right_list_top(),
            window.get_right_list_width(),
            window.get_right_list_height(),
        ),
    ];
    let pane = bounds.iter().position(|&(left, top, width, height)| {
        position.x >= left
            && position.x < left + width
            && position.y >= top
            && position.y < top + height
    })?;
    let (path, tabs) = if pane == 0 {
        (window.get_left_path(), window.get_left_tabs())
    } else {
        (window.get_right_path(), window.get_right_tabs())
    };
    Some(WheelTarget {
        pane,
        mode: if pane == 0 {
            window.get_left_view_mode()
        } else {
            window.get_right_view_mode()
        },
        path,
        tab: tabs
            .iter()
            .find(|tab| tab.active)
            .map(|tab| tab.identity)
            .unwrap_or_default(),
    })
}

impl InputState {
    fn active(&self) -> bool {
        self.pressed.active() || self.wheel.active()
    }

    /// Forget input ownership; the system's settings stay.
    fn reset(&mut self) {
        *self = Self {
            reduce_motion: self.reduce_motion,
            wheel_setting: self.wheel_setting,
            ..Self::default()
        };
    }

    /// Vertical distance of `lines` wheel lines over the list. A notch moves the
    /// viewers' distance (`notch_pixels_for`: the shared line distance scaled by the
    /// system's wheel setting, or one screen). A whole notch moves the rows nearest
    /// to it (at least one; one thumbnail band) and lands on a row boundary, so the
    /// idle row alignment has nothing left to move after it. Fractional lines
    /// (precision touchpads, high-resolution wheels) move their share of it.
    fn wheel_distance(&self, lines: f32) -> f32 {
        let notch = notch_pixels_for(self.wheel_setting, self.page[1]);
        let row = self.line_height;
        if lines == 0.0
            || lines.fract() != 0.0
            || !row.is_finite()
            || row <= 0.0
            || !self.content_y.is_finite()
        {
            return lines * notch;
        }
        let rows = (notch / row).round().max(1.0);
        // WheelMotion::push drops the remainder of a reversed glide.
        let pending = self.wheel.pending()[1] as f32;
        let base = self.content_y
            + if pending != 0.0 && pending.signum() == lines.signum() {
                pending
            } else {
                0.0
            };
        let landing = ((base + lines * rows * row) / row).round() * row;
        landing - base
    }

    #[cfg(test)]
    fn observe(&mut self, event: &WindowEvent, scale: f32, now: Duration) -> bool {
        self.observe_source(event, scale, now, false)
    }

    /// Only replace unphased line wheel input over a panel. The window's
    /// NativeScroll delivers touchpad pixel input and adds momentum; an unphased
    /// touchpad line stream (X11) is delivered here without wheel easing.
    fn observe_source(
        &mut self,
        event: &WindowEvent,
        scale: f32,
        now: Duration,
        touchpad: bool,
    ) -> bool {
        self.pressed.observe(event);
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some(*position);
            }
            WindowEvent::MouseWheel {
                delta: MouseScrollDelta::LineDelta(x, y),
                phase: TouchPhase::Moved,
                ..
            } if !self.pressed.active() => {
                let Some(cursor) = self.cursor else {
                    return false;
                };
                if !scale.is_finite() || scale <= 0.0 {
                    return false;
                }
                let cursor = cursor.to_logical::<f32>(f64::from(scale));
                self.wheel_position = Some(slint::LogicalPosition::new(cursor.x, cursor.y));
                if touchpad {
                    // Finger motion follows the finger at the shared line distance,
                    // as the window's kinetic tracker measures it.
                    return self
                        .wheel
                        .push_direct(x * LINE_PIXELS, y * LINE_PIXELS, now);
                }
                let x = x * notch_pixels_for(self.wheel_setting, self.page[0]);
                let y = self.wheel_distance(*y);
                if self.reduce_motion {
                    return self.wheel.push_direct(x, y, now);
                }
                return self.wheel.push(x, y, now);
            }
            WindowEvent::CursorLeft { .. }
            | WindowEvent::Focused(false)
            | WindowEvent::Destroyed => {
                self.wheel.cancel();
                self.cursor = None;
            }
            WindowEvent::MouseWheel { .. }
            | WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            }
            | WindowEvent::Touch(_)
            | WindowEvent::ScaleFactorChanged { .. } => self.wheel.cancel(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.wheel.cancel();
            }
            _ => {}
        }
        false
    }
}

fn wheel_timer(window: &MainWindow, state: &Rc<RefCell<InputState>>) -> Rc<slint::Timer> {
    let timer = Rc::new(slint::Timer::default());
    let weak_timer = Rc::downgrade(&timer);
    let timer_state = state.clone();
    let timer_window = window.as_weak();
    timer.start(slint::TimerMode::Repeated, WHEEL_FRAME, move || {
        let Some(timer) = weak_timer.upgrade() else {
            return;
        };
        let Some(window) = timer_window.upgrade() else {
            timer.stop();
            return;
        };
        if timer_state.borrow().wheel.active() {
            // A timer can run more than once before a native redraw. Only
            // wake rendering here; spend animation time on presented frames.
            window.window().request_redraw();
        } else {
            timer.stop();
        }
    });
    timer.stop();
    timer
}

fn wheel_frame(window: &MainWindow, state: &Rc<RefCell<InputState>>) -> bool {
    let (frame, position, active, moving) = {
        let mut state = state.borrow_mut();
        let current = state
            .wheel_position
            .and_then(|position| wheel_target(window, position));
        if current.is_none() || current != state.target {
            state.wheel.cancel();
        }
        let frame = state.wheel.tick(now(window));
        (
            frame,
            state.wheel_position,
            state.active(),
            state.wheel.active(),
        )
    };
    if let (Some((delta_x, delta_y)), Some(position)) = (frame, position) {
        // Native RedrawRequested dispatches this immediately before painting,
        // after checking the original list and popup ownership again.
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position,
                delta_x,
                delta_y,
            });
    }
    window.set_scroll_interaction_active(active);
    moving
}

fn observe(
    window: &MainWindow,
    state: &Rc<RefCell<InputState>>,
    timer: &slint::Timer,
    event: &WindowEvent,
    touchpad: bool,
) -> bool {
    // Winit's raw touch events have no public WindowEvent representation in the
    // pinned Slint hook. Capture before its Flickable can defer child down.
    if let WindowEvent::Touch(touch) = event {
        // The pinned runtime turns a cancelled tap into Released then Exit.
        // Invalidate before that synthetic release; clearing the snapshot would
        // instead permit the child to capture the current live context.
        if touch.phase == TouchPhase::Cancelled {
            window.invoke_cancel_function_press();
        }
        if touch.phase == TouchPhase::Started && !state.borrow().pressed.has_touches() {
            let point = touch
                .location
                .to_logical::<f32>(f64::from(window.window().scale_factor()));
            window.invoke_capture_function_press(point.x, point.y);
        } else if matches!(touch.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            let weak = window.as_weak();
            let state = state.clone();
            // Keep the snapshot through the runtime's release/click dispatch.
            // A newer press prevents an older deferred release from clearing it.
            slint::Timer::single_shot(Duration::ZERO, move || {
                if !state.borrow().pressed.active()
                    && let Some(window) = weak.upgrade()
                {
                    window.invoke_release_function_press();
                }
            });
        }
    }
    if matches!(event, WindowEvent::Focused(false) | WindowEvent::Destroyed) {
        window.invoke_cancel_function_press();
        window.invoke_release_function_press();
    }
    if matches!(event, WindowEvent::RedrawRequested) {
        if !wheel_frame(window, state) {
            timer.stop();
        }
        return false;
    }
    let (replaced, active, moving) = {
        let mut state = state.borrow_mut();
        let scale = window.window().scale_factor();
        // Hit-test this event's location, not the previously cached location.
        // Tiny movements inside the same list must not discard a wheel notch.
        if let WindowEvent::CursorMoved { position, .. } = event {
            state.cursor = Some(*position);
        }
        let target = state.cursor.and_then(|cursor| {
            let logical = cursor.to_logical::<f32>(f64::from(scale));
            wheel_target(window, slint::LogicalPosition::new(logical.x, logical.y))
        });
        if target.is_none() || state.target != target {
            state.wheel.cancel();
        }
        (state.line_height, state.content_y, state.page) =
            target.as_ref().map_or((0.0, 0.0, [0.0; 2]), |target| {
                if target.pane == 0 {
                    (
                        window.get_left_view_row_height(),
                        window.get_left_list_content_y(),
                        [window.get_left_list_width(), window.get_left_list_height()],
                    )
                } else {
                    (
                        window.get_right_view_row_height(),
                        window.get_right_list_content_y(),
                        [
                            window.get_right_list_width(),
                            window.get_right_list_height(),
                        ],
                    )
                }
            });
        state.target = target;
        let replaced = state.observe_source(event, scale, now(window), touchpad);
        if state.target.is_none() {
            state.wheel.cancel();
        }
        (
            replaced && state.wheel.active(),
            state.active(),
            state.wheel.active(),
        )
    };
    if moving && !timer.running() {
        timer.restart();
    } else if !moving {
        timer.stop();
    }
    window.set_scroll_interaction_active(active);
    window.set_touch_contact(state.borrow().pressed.has_touches());
    if replaced {
        // Fluent's scrollbar wheel handler omits scrolled. Release an explicit
        // cursor reveal before the first fractional panel frame changes content-y.
        // This hit-test runs once per native notch, never on the animation frames.
        let position = state.borrow().wheel_position;
        if let (
            Some(position),
            WindowEvent::MouseWheel {
                delta: MouseScrollDelta::LineDelta(x, y),
                ..
            },
        ) = (position, event)
        {
            midnight_ui_input::notify_scrollbar_wheel(
                window.window(),
                LogicalPoint::new(position.x, position.y),
                *x,
                *y,
            );
        }
        window.window().request_redraw();
    }
    replaced
}

fn now(window: &MainWindow) -> Duration {
    WindowInner::from_pub(window.window())
        .context()
        .platform()
        .duration_since_start()
}

pub(crate) fn install(window: &MainWindow) {
    let weak = window.as_weak();
    let state = Rc::new(RefCell::new(InputState::default()));
    let timer = wheel_timer(window, &state);
    let auxiliary_scroll = Rc::new(RefCell::new(NativeScroll::new(window)));
    let release_auxiliary_scroll = auxiliary_scroll.clone();
    let release_state = state.clone();
    let release_window = window.as_weak();
    window.on_release_drag_input(move || {
        let mut state = release_state.borrow_mut();
        state.pressed.release_left_button();
        release_auxiliary_scroll.borrow_mut().release_mouse_drag();
        state.wheel.cancel();
        if let Some(window) = release_window.upgrade() {
            window.set_scroll_interaction_active(state.active());
        }
    });
    let mut shortcuts = crate::native_shortcuts::ShortcutState::default();
    let mut kinetic_shown = false;
    let mut settings_read = false;
    window.window().on_winit_window_event(move |_, event| {
        let Some(window) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        // The system's motion and wheel settings, as the viewers read them: with the
        // first native event and whenever the window gains the focus (the user
        // changes them in another window), never per notch.
        if !settings_read || matches!(event, WindowEvent::Focused(true)) {
            settings_read = true;
            let reduce_motion = !crate::motion_preference::animations_enabled();
            let wheel_setting = crate::wheel_setting::system_wheel_setting();
            {
                let mut state = state.borrow_mut();
                state.reduce_motion = reduce_motion;
                state.wheel_setting = wheel_setting;
            }
            auxiliary_scroll
                .borrow_mut()
                .set_wheel_setting(wheel_setting);
            window.set_reduce_motion(reduce_motion);
        }
        if crate::dialog_host::dialog_input_blocked(window.window()) {
            timer.stop();
            auxiliary_scroll.borrow_mut().reset();
            state.borrow_mut().reset();
            shortcuts = crate::native_shortcuts::ShortcutState::default();
            if is_dialog_input(event) {
                return EventResult::PreventDefault;
            }
        }
        // The shared kinetic tracker sees every event first; panel routing below
        // delivers touchpad lines unchanged and momentum frames arrive as wheels.
        let source = auxiliary_scroll.borrow_mut().track(window.window(), event);
        let embedded_modal = window.get_embedded_modal_open();
        let replaced = if embedded_modal {
            window.global::<crate::PopupFocus>().invoke_update();
            // Keep native delivery available to the popup's editors/IME and
            // scrollbars while retiring background panel input ownership.
            timer.stop();
            state.borrow_mut().reset();
            window.set_scroll_interaction_active(false);
            false
        } else {
            crate::drag_bridge::observe_native(&window, event);
            observe(
                &window,
                &state,
                &timer,
                event,
                source == midnight_ui_input::WheelSource::Touchpad,
            )
        };
        let auxiliary_replaced = route_auxiliary(
            &window,
            &state,
            &auxiliary_scroll,
            event,
            replaced,
            &mut kinetic_shown,
        );
        // This callback runs only for a real Winit adapter. Software-window
        // tests retain the Slint logical-key fallback.
        window.set_native_shortcuts_authoritative(true);
        if shortcuts.handle(&window, event) || replaced || auxiliary_replaced {
            return EventResult::PreventDefault;
        }
        EventResult::Propagate
    });
}

/// After panel routing: auxiliary scrollbar/command-strip wheel easing and the
/// window's touchpad momentum. Returns true when the auxiliary path replaced input.
fn route_auxiliary(
    window: &MainWindow,
    state: &RefCell<InputState>,
    auxiliary_scroll: &RefCell<NativeScroll>,
    event: &WindowEvent,
    replaced: bool,
    kinetic_shown: &mut bool,
) -> bool {
    // Panel identity/routing gets first refusal. Only the remaining native
    // scrollbar and command-strip paths need auxiliary interpolation.
    let replaced_here = if replaced {
        auxiliary_scroll.borrow_mut().cancel_wheel();
        false
    } else {
        let command = (window.get_command_path_content_width() > window.get_command_path_width())
            .then(|| CommandStrip {
                bounds: LogicalRect::new(
                    LogicalPoint::new(
                        window.get_command_path_left(),
                        window.get_command_path_top(),
                    ),
                    LogicalSize::new(
                        window.get_command_path_width(),
                        window.get_command_path_height(),
                    ),
                ),
                path: window.get_command_directory(),
            });
        auxiliary_scroll
            .borrow_mut()
            .route(window.window(), event, command)
    };
    // Momentum is scroll interaction too: rows settle only once it stops.
    let kinetic = auxiliary_scroll.borrow().kinetic_active();
    if kinetic || *kinetic_shown {
        window.set_scroll_interaction_active(kinetic || state.borrow().active());
    }
    *kinetic_shown = kinetic;
    replaced_here
}

fn is_dialog_input(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::Ime(_)
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::PinchGesture { .. }
            | WindowEvent::PanGesture { .. }
            | WindowEvent::RotationGesture { .. }
            | WindowEvent::DoubleTapGesture { .. }
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::DroppedFile(_)
            | WindowEvent::HoveredFile(_)
            | WindowEvent::CloseRequested
    )
}

#[cfg(test)]
#[path = "scroll_input_pane_tests.rs"]
mod pane_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use slint::winit_030::winit::{
        dpi::PhysicalPosition,
        event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, Touch, TouchPhase},
    };

    fn mouse(button: MouseButton, state: ElementState) -> WindowEvent {
        WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button,
        }
    }

    fn touch(id: u64, phase: TouchPhase) -> WindowEvent {
        WindowEvent::Touch(Touch {
            device_id: DeviceId::dummy(),
            phase,
            location: PhysicalPosition::new(10.0, 10.0),
            force: None,
            id,
        })
    }

    #[test]
    fn held_mouse_buttons_block_settling_until_every_button_is_released() {
        let mut state = PressState::default();
        assert!(state.observe(&mouse(MouseButton::Left, ElementState::Pressed)));
        assert!(state.observe(&mouse(MouseButton::Right, ElementState::Pressed)));
        assert!(state.observe(&mouse(MouseButton::Left, ElementState::Released)));
        assert!(!state.observe(&mouse(MouseButton::Right, ElementState::Released)));
    }

    #[test]
    fn held_touches_remain_active_until_release_cancel_or_focus_loss() {
        let mut state = PressState::default();
        assert!(state.observe(&touch(7, TouchPhase::Started)));
        assert!(state.observe(&touch(8, TouchPhase::Started)));
        assert!(state.observe(&touch(7, TouchPhase::Ended)));
        assert!(!state.observe(&touch(8, TouchPhase::Cancelled)));
        assert!(state.observe(&touch(8, TouchPhase::Started)));
        assert!(!state.observe(&WindowEvent::Focused(false)));
        assert!(!state.observe(&touch(8, TouchPhase::Moved)));
    }

    #[test]
    fn phased_touchpad_holds_scroll_interaction_until_release() {
        let mut state = PressState::default();
        let wheel = |phase| WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -3.5)),
            phase,
        };
        assert!(!state.observe(&wheel(TouchPhase::Moved)));
        assert!(state.observe(&wheel(TouchPhase::Started)));
        assert!(state.observe(&wheel(TouchPhase::Moved)));
        assert!(!state.observe(&wheel(TouchPhase::Ended)));
    }

    fn cursor(x: f64, y: f64) -> WindowEvent {
        WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        }
    }

    fn wheel(delta: MouseScrollDelta, phase: TouchPhase) -> WindowEvent {
        WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta,
            phase,
        }
    }

    #[test]
    fn line_wheel_replacement_requires_a_pointer_and_preserves_logical_coordinates() {
        let mut state = InputState::default();
        let line = wheel(MouseScrollDelta::LineDelta(0.0, -1.0), TouchPhase::Moved);
        assert!(!state.observe(&line, 2.0, Duration::ZERO));
        state.observe(&cursor(120.0, 200.0), 2.0, Duration::ZERO);
        assert!(state.observe(&line, 2.0, Duration::ZERO));
        assert_eq!(
            state.wheel_position,
            Some(slint::LogicalPosition::new(60.0, 100.0))
        );
        assert!(state.active());
        let frame = state.wheel.tick(Duration::from_millis(16)).unwrap();
        assert!(frame.1 < 0.0 && frame.1 > -20.0);
        for millis in (2..=16).map(|frame| frame * 16) {
            state.wheel.tick(Duration::from_millis(millis));
        }
        assert!(!state.active());
    }

    #[test]
    fn pixel_and_phased_line_gestures_keep_the_native_backend_path() {
        let mut state = InputState::default();
        state.observe(&cursor(120.0, 200.0), 1.0, Duration::ZERO);
        let line = wheel(MouseScrollDelta::LineDelta(0.0, -1.0), TouchPhase::Moved);
        assert!(state.observe(&line, 1.0, Duration::ZERO));
        let pixel = wheel(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -3.5)),
            TouchPhase::Moved,
        );
        assert!(!state.observe(&pixel, 1.0, Duration::ZERO));
        assert!(!state.wheel.active());
        assert!(!state.observe(
            &wheel(MouseScrollDelta::LineDelta(0.0, 0.0), TouchPhase::Started),
            1.0,
            Duration::ZERO,
        ));
        assert!(!state.observe(&line, 1.0, Duration::ZERO));
        assert!(state.active(), "a held touchpad blocks row settling");
        state.observe(
            &wheel(MouseScrollDelta::LineDelta(0.0, 0.0), TouchPhase::Ended),
            1.0,
            Duration::ZERO,
        );
        assert!(!state.active());
    }

    #[test]
    fn pointer_exit_press_and_focus_loss_stop_old_wheel_frames() {
        for interrupt in [
            WindowEvent::CursorLeft {
                device_id: DeviceId::dummy(),
            },
            mouse(MouseButton::Left, ElementState::Pressed),
            WindowEvent::Focused(false),
        ] {
            let mut state = InputState::default();
            state.observe(&cursor(120.0, 200.0), 1.0, Duration::ZERO);
            state.observe(
                &wheel(MouseScrollDelta::LineDelta(0.0, -1.0), TouchPhase::Moved),
                1.0,
                Duration::ZERO,
            );
            assert!(state.wheel.active());
            state.observe(&interrupt, 1.0, Duration::from_millis(20));
            assert!(!state.wheel.active());
            assert_eq!(state.wheel.tick(Duration::from_secs(1)), None);
        }
    }

    #[test]
    fn moving_the_pointer_preserves_motion_until_target_routing_decides_otherwise() {
        let mut state = InputState::default();
        state.observe(&cursor(120.0, 200.0), 1.0, Duration::ZERO);
        state.observe(
            &wheel(MouseScrollDelta::LineDelta(0.0, -1.0), TouchPhase::Moved),
            1.0,
            Duration::ZERO,
        );
        state.observe(&cursor(121.0, 201.0), 1.0, Duration::from_millis(1));
        assert!(state.wheel.active());
        assert_eq!(state.cursor, Some(PhysicalPosition::new(121.0, 201.0)));
        let total: f32 = (1..=16)
            .map(|frame| frame * 16)
            .filter_map(|millis| state.wheel.tick(Duration::from_millis(millis)))
            .map(|(_, y)| y)
            .sum();
        // One notch at the default wheel setting, away from any panel.
        let notch = notch_pixels_for(WheelSetting::default(), 0.0);
        assert!((total + notch).abs() < 0.001);
    }
}
