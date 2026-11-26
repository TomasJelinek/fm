//! Native input for the acceptance suite and opt-in performance probes
//! (feature `acceptance-seams`; the application never compiles this module).
//!
//! The suite drives viewers on Slint's software platform, where winit never runs, so the
//! window's native observer (the closure winit's events reach through
//! `on_winit_window_event`) would see nothing: wheel notches converted by the system's
//! wheel setting, touchpad reports, reduced motion, the inactive selection colours and an
//! HTML page's pointer. [`wheel`], [`pointer`] and [`focus`] hand that same closure the
//! events winit would deliver and give Slint what the observer leaves alone, as winit's
//! backend does. The frames on which wheel
//! motion and touchpad momentum step (winit's `RedrawRequested`) follow by themselves while
//! the window moves, also when it moves on after waiting for the backend.
//! [`override_settings`] replaces the system settings the viewer reads, for this process.
//! The panels' seam in `midnight-ui` drives the panels the same way.
use super::*;
use i_slint_core::{
    input::{BackendMouseEvent, TouchPhase as SlintPhase},
    lengths::LogicalPoint,
    window::WindowInner,
};
use slint::winit_030::winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent},
};
#[cfg(not(test))]
use std::sync::{Mutex, PoisonError};

pub use midnight_ui_input::WheelSetting;

/// Logical pixels Slint's winit backend gives one wheel line that the observer leaves alone.
const SLINT_LINE: f32 = 60.0;
/// Frames keep coming this long after a seam input, so that motion it starts is seen.
const SETTLE: Duration = Duration::from_millis(250);

/// One native wheel report, in winit's units and signs: a positive `y` scrolls up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NativeWheel {
    /// A mouse wheel, in lines: one notch is 1.0, a high-resolution wheel or a Windows
    /// precision touchpad reports fractions.
    Lines { x: f32, y: f32 },
    /// A touchpad, in physical pixels, within its gesture.
    Pixels { x: f64, y: f64, phase: NativePhase },
}

/// Where a touchpad report falls in its gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativePhase {
    Started,
    Moved,
    Ended,
}

/// The system settings a viewer reads; `None` reads the system's (the wheel) or the host's
/// `ViewerWindows::set_motion_preference` (reduced motion). A window reads them with its
/// first native event and whenever it gains the focus, so a change applies after
/// [`focus`]`(…, true)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SystemSettings {
    /// Lines per notch, or one screen per notch (`WheelSetting::Page`).
    pub wheel: Option<WheelSetting>,
    /// The system asks for less motion (Windows: animation effects off).
    pub reduce_motion: Option<bool>,
}

const SYSTEM: SystemSettings = SystemSettings {
    wheel: None,
    reduce_motion: None,
};

#[cfg(not(test))]
static SETTINGS: Mutex<SystemSettings> = Mutex::new(SYSTEM);

// This crate's own tests run in parallel threads, each with its own windows: each thread
// keeps its own override, so that one test's override never reaches another's windows.
#[cfg(test)]
thread_local! {
    static SETTINGS: RefCell<SystemSettings> = const { RefCell::new(SYSTEM) };
}

/// Restores the settings in force before its override when dropped; guards drop in the
/// reverse order of their creation.
#[must_use = "the override ends when the guard drops"]
pub struct SettingsGuard {
    previous: SystemSettings,
}

impl Drop for SettingsGuard {
    fn drop(&mut self) {
        settings(|settings| *settings = self.previous);
    }
}

#[cfg(not(test))]
fn settings<T>(apply: impl FnOnce(&mut SystemSettings) -> T) -> T {
    apply(&mut SETTINGS.lock().unwrap_or_else(PoisonError::into_inner))
}

#[cfg(test)]
fn settings<T>(apply: impl FnOnce(&mut SystemSettings) -> T) -> T {
    SETTINGS.with(|settings| apply(&mut settings.borrow_mut()))
}

/// Read `replacement` instead of the system settings in this process until the guard drops.
pub fn override_settings(replacement: SystemSettings) -> SettingsGuard {
    SettingsGuard {
        previous: settings(|settings| std::mem::replace(settings, replacement)),
    }
}

pub(crate) fn wheel_override() -> Option<WheelSetting> {
    settings(|settings| settings.wheel)
}

pub(crate) fn reduce_motion_override() -> Option<bool> {
    settings(|settings| settings.reduce_motion)
}

/// What the seams keep for one viewer window.
#[derive(Default)]
pub(crate) struct Seam {
    observer: RefCell<Option<NativeObserver>>,
    /// The observer's wheel and touchpad state: its momentum wants frames.
    scroll: RefCell<Option<Rc<RefCell<midnight_ui_input::NativeScroll>>>>,
    /// The frame pump (`frames`).
    frames: Timer,
    last_input: Cell<Duration>,
    /// The frames the pump sent.
    #[cfg(test)]
    sent: Cell<u64>,
}

/// Keep `open`'s native observer and its wheel state (`bind`).
pub(crate) fn install(
    open: &OpenWindow,
    observer: NativeObserver,
    scroll: Rc<RefCell<midnight_ui_input::NativeScroll>>,
) {
    open.seam.observer.replace(Some(observer));
    open.seam.scroll.replace(Some(scroll));
}

/// `window`'s state and native observer, when `viewers` opened it and it is shown.
fn observer(
    viewers: &ViewerWindows,
    window: &ViewerWindow,
) -> Option<(Rc<OpenWindow>, NativeObserver)> {
    let open = viewers
        .windows
        .borrow()
        .iter()
        .find(|open| std::ptr::eq(open.window.window(), window.window()))
        .cloned()?;
    if open.closed.get() || !open.window.window().is_visible() {
        return None;
    }
    let observer = open.seam.observer.borrow().clone()?;
    Some((open, observer))
}

/// Whether the observer consumed `event`, as `on_winit_window_event` asks it.
fn deliver(observer: &NativeObserver, event: &WindowEvent) -> bool {
    // As the window's native event hook: user input postpones the spare viewer component.
    note_user_input_event(event);
    observer
        .try_borrow_mut()
        .is_ok_and(|mut observe| (*observe)(event) == EventResult::PreventDefault)
}

/// Whether `open` still moves: its wheel glides, camera or touchpad momentum, which step
/// on each frame. A glide that waits for the backend does not move meanwhile (`frames`).
fn moving(open: &OpenWindow) -> bool {
    open.raster_redraw.running()
        || open.image_redraw.running()
        || web_input::gliding(open)
        || open.seam.scroll.borrow().as_ref().is_some_and(|scroll| {
            scroll
                .try_borrow()
                .is_ok_and(|scroll| scroll.kinetic_active())
        })
}

/// A seam input reached `open`: winit's frames (`RedrawRequested`, every wheel frame) follow
/// for a while after the input and whenever the window moves. Winit gives a frame for each
/// redraw the window asks for, which it does while it moves; a glide that waits for the
/// backend asks for none meanwhile and goes on without new input when the reply comes (a
/// glide handed over to a corrected surface, `raster_rebase`). So the pump runs until the
/// window closes and sends nothing while the window rests. It used to stop at rest, also
/// while such a glide waited, and the rest of that glide never got a frame: one notch up
/// after a scrollbar drag moved 15 to 42 px of 100 (acceptance VIEW-NOTCH-UP).
fn frames(open: &Rc<OpenWindow>) {
    // Native probes must measure compositor-paced frames, not this headless pump.
    if open.window.window().has_winit_window() {
        return;
    }
    open.seam.last_input.set(image_motion::now(open));
    if open.seam.frames.running() {
        return;
    }
    let weak = Rc::downgrade(open);
    open.seam.frames.start(
        TimerMode::Repeated,
        midnight_ui_input::WHEEL_FRAME,
        move || {
            let Some(open) = weak.upgrade() else {
                return;
            };
            let observer = open.seam.observer.borrow().clone();
            let Some(observer) = observer.filter(|_| !open.closed.get()) else {
                open.seam.frames.stop();
                return;
            };
            let settling =
                image_motion::now(&open).saturating_sub(open.seam.last_input.get()) < SETTLE;
            if !settling && !moving(&open) {
                return;
            }
            #[cfg(test)]
            open.seam.sent.set(open.seam.sent.get() + 1);
            if !deliver(&observer, &WindowEvent::RedrawRequested) {
                // What winit's backend then does: draw the window.
                open.window.window().request_redraw();
            }
        },
    );
}

/// Move the pointer to `at` (logical) and turn `wheel` there, as winit reports both to
/// `window`: through its native observer first, then what that leaves alone to Slint as
/// winit's backend converts it (lines × 60 or physical pixels ÷ scale, with the report's
/// phase). `Some(consumed)`, or `None` when `viewers` did not open `window` or it is hidden.
pub fn wheel(
    viewers: &ViewerWindows,
    window: &ViewerWindow,
    at: slint::LogicalPosition,
    wheel: NativeWheel,
) -> Option<bool> {
    let (open, observer) = observer(viewers, window)?;
    let target = open.window.window();
    let scale = f64::from(target.scale_factor());
    let position = LogicalPoint::new(at.x, at.y);
    let moved = WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(f64::from(at.x) * scale, f64::from(at.y) * scale),
    };
    if !deliver(&observer, &moved) {
        target.dispatch_event(slint::platform::WindowEvent::internal(
            BackendMouseEvent::Moved {
                position,
                touch_finger_id: 0,
            },
        ));
    }
    let (delta, phase, forwarded) = match wheel {
        NativeWheel::Lines { x, y } => (
            MouseScrollDelta::LineDelta(x, y),
            TouchPhase::Moved,
            (x * SLINT_LINE, y * SLINT_LINE),
        ),
        NativeWheel::Pixels { x, y, phase } => (
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(x, y)),
            match phase {
                NativePhase::Started => TouchPhase::Started,
                NativePhase::Moved => TouchPhase::Moved,
                NativePhase::Ended => TouchPhase::Ended,
            },
            ((x / scale) as f32, (y / scale) as f32),
        ),
    };
    let event = WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta,
        phase,
    };
    let consumed = deliver(&observer, &event);
    if !consumed {
        // As winit's backend: `winit_touch_phase`.
        let phase = match phase {
            TouchPhase::Started => SlintPhase::Started,
            TouchPhase::Moved => SlintPhase::Moved,
            TouchPhase::Ended => SlintPhase::Ended,
            TouchPhase::Cancelled => SlintPhase::Cancelled,
        };
        target.dispatch_event(slint::platform::WindowEvent::internal(
            BackendMouseEvent::Wheel {
                position,
                delta_x: forwarded.0,
                delta_y: forwarded.1,
                phase,
            },
        ));
    }
    frames(&open);
    Some(consumed)
}

/// One native pointer report: the pointer moves to a point, or the left button goes down
/// or up there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativePointer {
    Move,
    Press,
    Release,
}

/// Move the pointer to `at` (logical) and, for [`NativePointer::Press`] and
/// [`NativePointer::Release`], change the left button there, as winit reports them to
/// `window`: through its native observer first (an HTML page's browser reads the pointer
/// only there), then what that leaves alone to Slint as winit's backend gives it.
/// `Some(consumed)` for the last event, or `None` when `viewers` did not open `window` or
/// it is hidden.
pub fn pointer(
    viewers: &ViewerWindows,
    window: &ViewerWindow,
    at: slint::LogicalPosition,
    action: NativePointer,
) -> Option<bool> {
    let (open, observer) = observer(viewers, window)?;
    let target = open.window.window();
    let scale = f64::from(target.scale_factor());
    let moved = WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(f64::from(at.x) * scale, f64::from(at.y) * scale),
    };
    let mut consumed = deliver(&observer, &moved);
    if !consumed {
        target.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
    }
    let state = match action {
        NativePointer::Move => None,
        NativePointer::Press => Some(ElementState::Pressed),
        NativePointer::Release => Some(ElementState::Released),
    };
    if let Some(state) = state {
        let event = WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        };
        consumed = deliver(&observer, &event);
        if !consumed {
            let button = slint::platform::PointerEventButton::Left;
            target.dispatch_event(if state == ElementState::Pressed {
                slint::platform::WindowEvent::PointerPressed {
                    position: at,
                    button,
                }
            } else {
                slint::platform::WindowEvent::PointerReleased {
                    position: at,
                    button,
                }
            });
        }
    }
    frames(&open);
    Some(consumed)
}

/// `window` gains (`true`) or loses the system focus, as winit's `Focused` reports it:
/// through its native observer (the inactive selection colours; gaining it reads the
/// settings again), then Slint's activation when it changes, as winit's backend gives it.
/// `None` when `viewers` did not open `window` or it is hidden.
pub fn focus(viewers: &ViewerWindows, window: &ViewerWindow, focused: bool) -> Option<()> {
    let (open, observer) = observer(viewers, window)?;
    if !deliver(&observer, &WindowEvent::Focused(focused))
        && WindowInner::from_pub(open.window.window()).active() != focused
    {
        open.window
            .window()
            .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(focused));
    }
    frames(&open);
    Some(())
}

#[cfg(test)]
#[path = "acceptance_seams_tests.rs"]
mod tests;
