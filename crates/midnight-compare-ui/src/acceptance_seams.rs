//! Native input for the acceptance suite only (feature `acceptance-seams`, which only
//! `crates/midnight-acceptance` enables; the application never compiles this module).
//!
//! The suite drives comparison windows on Slint's software platform, where winit never
//! runs, so the window's native observer (the closure winit's events reach through
//! `on_winit_window_event`) would see nothing: no wheel notch, which only that observer
//! converts with the system's wheel setting and glides. [`wheel`] hands that same closure
//! the events winit would deliver, and gives Slint what the observer leaves alone, as
//! winit's backend does. The frames on which the glide steps (winit's `RedrawRequested`)
//! follow by themselves for a while after each input, as the viewers' seam sends them
//! (`midnight_viewer_ui::acceptance_seams`).
use super::*;
use i_slint_core::{
    input::{BackendMouseEvent, TouchPhase as SlintPhase},
    lengths::LogicalPoint,
    window::WindowInner,
};
use slint::winit_030::winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
};

/// Logical pixels Slint's winit backend gives one wheel line that the observer leaves alone.
const SLINT_LINE: f32 = 60.0;
/// Frames keep coming this long after a seam input: a glide, at the longest Scrolling
/// glide (four times 200 ms), and the view's settling (`SETTLE`).
const FRAMES_FOR: Duration = Duration::from_millis(1200);

/// What the seam keeps for one comparison window.
#[derive(Default)]
pub(crate) struct Seam {
    pub(crate) observer: RefCell<Option<NativeObserver>>,
    frames: Timer,
    last_input: Cell<Duration>,
}

fn now(window: &slint::Window) -> Duration {
    WindowInner::from_pub(window)
        .context()
        .platform()
        .duration_since_start()
}

/// Whether the observer consumed `event`, as `on_winit_window_event` asks it.
fn deliver(observer: &NativeObserver, event: &WindowEvent) -> bool {
    observer
        .try_borrow_mut()
        .is_ok_and(|mut observe| (*observe)(event) == EventResult::PreventDefault)
}

/// Winit's frames for a while after an input: `RedrawRequested` every wheel frame.
fn frames(open: &Rc<OpenWindow>) {
    open.seam.last_input.set(now(open.window.window()));
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
            if now(open.window.window()).saturating_sub(open.seam.last_input.get()) > FRAMES_FOR {
                open.seam.frames.stop();
                return;
            }
            if !deliver(&observer, &WindowEvent::RedrawRequested) {
                open.window.window().request_redraw();
            }
        },
    );
}

/// Move the pointer to `at` (logical) and turn the mouse wheel there by `x` and `y` lines
/// (winit's units and signs: a positive `y` scrolls up), as winit reports both to `window`:
/// through its native observer first, then what that leaves alone to Slint as winit's
/// backend converts it. `Some(consumed)`, or `None` when `comparators` did not open
/// `window` or it is closed.
pub fn wheel(
    comparators: &ComparatorWindows,
    window: &ComparatorWindow,
    at: slint::LogicalPosition,
    x: f32,
    y: f32,
) -> Option<bool> {
    let open = comparators
        .windows
        .borrow()
        .iter()
        .find(|open| std::ptr::eq(open.window.window(), window.window()))
        .cloned()?;
    if open.closed.get() {
        return None;
    }
    let observer = open.seam.observer.borrow().clone()?;
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
    let event = WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::LineDelta(x, y),
        phase: TouchPhase::Moved,
    };
    let consumed = deliver(&observer, &event);
    if !consumed {
        target.dispatch_event(slint::platform::WindowEvent::internal(
            BackendMouseEvent::Wheel {
                position,
                delta_x: x * SLINT_LINE,
                delta_y: y * SLINT_LINE,
                phase: SlintPhase::Moved,
            },
        ));
    }
    frames(&open);
    Some(consumed)
}
