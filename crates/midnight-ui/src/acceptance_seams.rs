//! Native-equivalent input for the acceptance harness (feature `acceptance-seams`, which
//! only `midnight-acceptance` enables; the application never compiles this module).
//!
//! The harness renders the real MainWindow in software windows, where no Winit adapter
//! delivers native events. The panels take a wheel notch, Windows' lines per notch and
//! its animation setting only from native events, through the observer closure that
//! `on_winit_window_event` calls. This seam hands native-equivalent events to that same
//! stored closure, forwards what the observer leaves alone to Slint as Winit's backend
//! does, and sends the `RedrawRequested` that Winit sends before each frame. See
//! docs/developer/SCROLLING.md ("Acceptance seam").
use crate::{MainWindow, scroll_input::NativeObserver};
use i_slint_core::{
    input::{BackendMouseEvent, TouchPhase as SlintPhase},
    lengths::LogicalPoint,
    window::WindowInner,
};
use midnight_ui_input::WHEEL_FRAME;
pub use midnight_ui_input::WheelSetting;
use slint::{
    ComponentHandle,
    platform::WindowEvent as SlintEvent,
    winit_030::{
        EventResult,
        winit::{
            dpi::PhysicalPosition,
            event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
        },
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Mutex, PoisonError},
    time::Duration,
};

/// A native wheel report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NativeWheel {
    /// A mouse wheel, or a Windows precision touchpad, in lines: 1.0 is one notch and
    /// fractions are allowed. Winit's signs: +y scrolls up.
    Lines { x: f32, y: f32 },
    /// A touchpad's delta in physical pixels, with its gesture phase.
    Pixels { x: f64, y: f64, phase: NativePhase },
}

/// The phase of a touchpad gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativePhase {
    Started,
    Moved,
    Ended,
}

impl From<NativePhase> for TouchPhase {
    fn from(phase: NativePhase) -> Self {
        match phase {
            NativePhase::Started => Self::Started,
            NativePhase::Moved => Self::Moved,
            NativePhase::Ended => Self::Ended,
        }
    }
}

/// The system settings the panels read; `None` reads the system.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SystemSettings {
    /// Lines per notch, or one screen per notch (Windows: "lines to scroll at a time").
    pub wheel: Option<WheelSetting>,
    /// Less motion (Windows: "Animation effects" off).
    pub reduce_motion: Option<bool>,
}

/// Ends an [`override_settings`] override when dropped; the previous one returns.
#[must_use = "the override ends when the guard is dropped"]
#[derive(Debug)]
pub struct SettingsGuard {
    previous: SystemSettings,
}

static OVERRIDE: Mutex<SystemSettings> = Mutex::new(SystemSettings {
    wheel: None,
    reduce_motion: None,
});

/// Override the system settings the panels read, for the whole test process, until the
/// guard drops (guards end in the reverse order of their creation). As in production,
/// the panels read the settings with their first native event and whenever the window
/// gains the focus, so a scenario calls [`focus`]`(window, true)` after overriding.
pub fn override_settings(settings: SystemSettings) -> SettingsGuard {
    let mut current = OVERRIDE.lock().unwrap_or_else(PoisonError::into_inner);
    SettingsGuard {
        previous: std::mem::replace(&mut *current, settings),
    }
}

impl Drop for SettingsGuard {
    fn drop(&mut self) {
        *OVERRIDE.lock().unwrap_or_else(PoisonError::into_inner) = self.previous;
    }
}

/// The current override, which the panels' settings readers consult first.
pub(crate) fn overridden() -> SystemSettings {
    *OVERRIDE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Deliver a native wheel report at `at` (logical pixels) to `window`'s panels: a
/// `CursorMoved` to `at`, then the `MouseWheel`, through the same observer closure that
/// `on_winit_window_event` calls. What the observer leaves alone reaches Slint as Winit's
/// backend converts it: a pointer move, then a wheel of lines × 60 or physical pixels ÷
/// scale, with its phase (public form: `PointerMoved`, `PointerScrolled`). Returns
/// whether the observer consumed the wheel, or `None` when the window is not open.
pub fn wheel(window: &MainWindow, at: slint::LogicalPosition, wheel: NativeWheel) -> Option<bool> {
    let seam = Seam::open(window)?;
    let scale = window.window().scale_factor();
    let position = LogicalPoint::new(at.x, at.y);
    let cursor = WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(f64::from(at.x * scale), f64::from(at.y * scale)),
    };
    if !seam.deliver(window, &cursor)? {
        dispatch(
            window,
            BackendMouseEvent::Moved {
                position,
                touch_finger_id: 0,
            },
        );
    }
    let (delta, phase, forwarded) = match wheel {
        NativeWheel::Lines { x, y } => (
            MouseScrollDelta::LineDelta(x, y),
            TouchPhase::Moved,
            (x * 60.0, y * 60.0),
        ),
        NativeWheel::Pixels { x, y, phase } => (
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(x, y)),
            phase.into(),
            ((x / f64::from(scale)) as f32, (y / f64::from(scale)) as f32),
        ),
    };
    let event = WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta,
        phase,
    };
    let consumed = seam.deliver(window, &event)?;
    if !consumed {
        dispatch(
            window,
            BackendMouseEvent::Wheel {
                position,
                delta_x: forwarded.0,
                delta_y: forwarded.1,
                phase: slint_phase(phase),
            },
        );
    }
    seam.drive(window);
    Some(consumed)
}

/// Deliver a native focus change to `window`: `Focused` through the same observer
/// closure (a gained focus re-reads the system settings, as in production), then
/// Slint's `WindowActiveChanged` when the window's activation changes, as Winit's
/// backend sends it. `None` when the window is not open.
pub fn focus(window: &MainWindow, focused: bool) -> Option<()> {
    let seam = Seam::open(window)?;
    if !seam.deliver(window, &WindowEvent::Focused(focused))?
        && WindowInner::from_pub(window.window()).active() != focused
    {
        window
            .window()
            .dispatch_event(SlintEvent::WindowActiveChanged(focused));
    }
    seam.drive(window);
    Some(())
}

/// Frames keep following native input this long after the last seam event: longer
/// than the 200 ms wheel glide of scrollbars and the command strip, which do not mark
/// scroll interaction.
const FOLLOW: Duration = Duration::from_millis(250);

/// One window's stored observer and the timer that sends it frames.
#[derive(Clone)]
struct Seam {
    window: slint::Weak<MainWindow>,
    observer: NativeObserver,
    frames: Rc<slint::Timer>,
    last_input: Rc<Cell<Duration>>,
}

thread_local! {
    // Slint windows live on one thread; so does their observer.
    static SEAMS: RefCell<Vec<Seam>> = const { RefCell::new(Vec::new()) };
}

/// Keep `window`'s observer closure (from `scroll_input::install`) for the seam.
pub(crate) fn store(window: &MainWindow, observer: NativeObserver) {
    SEAMS.with(|seams| {
        let mut seams = seams.borrow_mut();
        seams.retain(|seam| seam.window.upgrade().is_some());
        seams.push(Seam {
            window: window.as_weak(),
            observer,
            frames: Rc::default(),
            last_input: Rc::default(),
        });
    });
}

impl Seam {
    /// The stored observer of `window`, when the window is shown.
    fn open(window: &MainWindow) -> Option<Self> {
        if !window.window().is_visible() {
            return None;
        }
        SEAMS.with(|seams| {
            seams
                .borrow()
                .iter()
                .find(|seam| {
                    seam.window
                        .upgrade()
                        .is_some_and(|stored| std::ptr::eq(stored.window(), window.window()))
                })
                .cloned()
        })
    }

    /// Hand one native event to the observer: whether it consumed the event.
    fn deliver(&self, window: &MainWindow, event: &WindowEvent) -> Option<bool> {
        deliver(&self.observer, window, event)
    }

    /// Winit sends `RedrawRequested` before each frame it presents, and the panels
    /// advance their wheel glide and touchpad momentum on it. Software windows get
    /// none, so send one through the same observer each wheel frame while scroll
    /// interaction lasts and for [`FOLLOW`] after the last seam event.
    fn drive(&self, window: &MainWindow) {
        self.last_input.set(now(window));
        if self.frames.running() {
            return;
        }
        let (weak, observer, last_input) = (
            self.window.clone(),
            self.observer.clone(),
            self.last_input.clone(),
        );
        let frames = Rc::downgrade(&self.frames);
        self.frames
            .start(slint::TimerMode::Repeated, WHEEL_FRAME, move || {
                let Some(frames) = frames.upgrade() else {
                    return;
                };
                let Some(window) = weak.upgrade() else {
                    frames.stop();
                    return;
                };
                if deliver(&observer, &window, &WindowEvent::RedrawRequested) == Some(false) {
                    // What Winit's backend then does: draw the window.
                    window.window().request_redraw();
                }
                if !window.get_scroll_interaction_active()
                    && now(&window).saturating_sub(last_input.get()) >= FOLLOW
                {
                    frames.stop();
                }
            });
    }
}

/// Hand one native event to `observer`, as `on_winit_window_event` does: whether it
/// consumed the event (`None` while the observer runs).
fn deliver(observer: &NativeObserver, window: &MainWindow, event: &WindowEvent) -> Option<bool> {
    let mut observe = observer.try_borrow_mut().ok()?;
    Some(matches!(
        (*observe)(window.window(), event),
        EventResult::PreventDefault
    ))
}

/// Seam tests share the process-wide settings override: they run one at a time.
#[cfg(test)]
pub(crate) fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn dispatch(window: &MainWindow, event: BackendMouseEvent) {
    window.window().dispatch_event(SlintEvent::internal(event));
}

/// Winit's backend conversion (`winit_touch_phase`).
fn slint_phase(phase: TouchPhase) -> SlintPhase {
    match phase {
        TouchPhase::Started => SlintPhase::Started,
        TouchPhase::Moved => SlintPhase::Moved,
        TouchPhase::Ended => SlintPhase::Ended,
        TouchPhase::Cancelled => SlintPhase::Cancelled,
    }
}

fn now(window: &MainWindow) -> Duration {
    WindowInner::from_pub(window.window())
        .context()
        .platform()
        .duration_since_start()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_nest_and_end_in_reverse_order() {
        let _serial = serial();
        let outer = override_settings(SystemSettings {
            wheel: Some(WheelSetting::Lines(1)),
            reduce_motion: None,
        });
        assert_eq!(
            crate::wheel_setting::system_wheel_setting(),
            WheelSetting::Lines(1)
        );
        let inner = override_settings(SystemSettings {
            wheel: Some(WheelSetting::Page),
            reduce_motion: Some(true),
        });
        assert_eq!(
            crate::wheel_setting::system_wheel_setting(),
            WheelSetting::Page
        );
        assert!(!crate::motion_preference::animations_enabled());
        drop(inner);
        assert_eq!(
            overridden(),
            SystemSettings {
                wheel: Some(WheelSetting::Lines(1)),
                reduce_motion: None,
            }
        );
        drop(outer);
        assert_eq!(overridden(), SystemSettings::default());
    }
}
