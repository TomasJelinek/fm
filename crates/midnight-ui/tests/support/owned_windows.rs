//! Installed UI input routes to embedded popups or independent native tools.
#![allow(dead_code)]
use super::ui::TestUi;
use i_slint_core::window::{WindowInner, WindowKind};
use slint::{
    ComponentHandle,
    platform::{
        Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

pub type Windows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;
struct Boundary(Windows, RepaintBufferType);
impl Platform for Boundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let native = MinimalSoftwareWindow::new(self.1);
        self.0.borrow_mut().push(native.clone());
        Ok(native)
    }
}
/// [`Boundary`] whose Slint timers and animations only follow a virtual clock.
struct ClockBoundary(Boundary, Rc<Cell<Duration>>);
impl Platform for ClockBoundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        self.0.create_window_adapter()
    }
    fn duration_since_start(&self) -> Duration {
        self.1.get()
    }
}
pub fn new() -> (TestUi, Windows) {
    with_buffer(RepaintBufferType::ReusedBuffer)
}
/// [`new`] whose Slint timers and animations only follow `clock`.
pub fn with_clock(clock: Rc<Cell<Duration>>) -> (TestUi, Windows) {
    clocked(clock, RepaintBufferType::ReusedBuffer, true)
}
/// [`with_clock`] for harnesses that render every frame into a new buffer
/// (`RepaintBufferType::NewBuffer`, as `modeless::draw` expects).
pub fn with_clock_and_buffer(
    clock: Rc<Cell<Duration>>,
    buffer: RepaintBufferType,
) -> (TestUi, Windows) {
    clocked(clock, buffer, true)
}
pub fn with_buffer(buffer: RepaintBufferType) -> (TestUi, Windows) {
    let windows = Windows::default();
    slint::platform::set_platform(Box::new(Boundary(windows.clone(), buffer))).unwrap();
    (window(&windows, true), windows)
}
/// [`with_clock`] without preinstalled widget callbacks, for tests that install the
/// production initializer (a Presenter) themselves.
pub fn bare_with_clock(clock: Rc<Cell<Duration>>) -> (TestUi, Windows) {
    clocked(clock, RepaintBufferType::ReusedBuffer, false)
}
/// Only the platform of [`with_clock`], for tests that create their own main windows
/// (several in turn, if they like); it records every adapter, newest last.
pub fn clock_platform(clock: Rc<Cell<Duration>>) -> Windows {
    install_clocked(clock, RepaintBufferType::ReusedBuffer)
}
fn install_clocked(clock: Rc<Cell<Duration>>, buffer: RepaintBufferType) -> Windows {
    let windows = Windows::default();
    let boundary = Boundary(windows.clone(), buffer);
    slint::platform::set_platform(Box::new(ClockBoundary(boundary, clock))).unwrap();
    windows
}
fn clocked(
    clock: Rc<Cell<Duration>>,
    buffer: RepaintBufferType,
    widget_callbacks: bool,
) -> (TestUi, Windows) {
    let windows = install_clocked(clock, buffer);
    (window(&windows, widget_callbacks), windows)
}
fn window(windows: &Windows, widget_callbacks: bool) -> TestUi {
    let window = midnight_ui::MainWindow::new().unwrap();
    window.set_preferences_ready(true);
    if widget_callbacks {
        midnight_ui::install_file_keys(&window);
    }
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let native = windows.borrow()[0].clone();
    let mut ui = TestUi {
        window,
        native,
        pixels: Vec::new(),
        width: 1340,
        height: 820,
    };
    ui.resize(1340, 820);
    ui
}
pub fn child_key(windows: &Windows, text: impl Into<slint::SharedString>) {
    slint::platform::update_timers_and_animations();
    let main = windows.borrow().first().expect("main adapter").clone();
    let embedded = WindowInner::from_pub(main.window())
        .active_popups()
        .iter()
        .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip));
    let target = if embedded {
        main
    } else {
        let children: Vec<_> = windows
            .borrow()
            .iter()
            .skip(1)
            .filter(|window| window.window().is_visible())
            .cloned()
            .collect();
        assert_eq!(
            children.len(),
            1,
            "exactly one independent tool must receive input"
        );
        children[0].clone()
    };
    target
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let text = text.into();
    target
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    target
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
