//! Installed UI input routes to embedded popups or independent native tools.
#![allow(dead_code)]
use crate::ui::TestUi;
use i_slint_core::window::{WindowInner, WindowKind};
use slint::{
    ComponentHandle,
    platform::{
        Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, rc::Rc};

pub type Windows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;
struct Boundary(Windows, RepaintBufferType);
impl Platform for Boundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let native = MinimalSoftwareWindow::new(self.1);
        self.0.borrow_mut().push(native.clone());
        Ok(native)
    }
}
pub fn new() -> (TestUi, Windows) {
    with_buffer(RepaintBufferType::ReusedBuffer)
}
pub fn with_buffer(buffer: RepaintBufferType) -> (TestUi, Windows) {
    let windows = Windows::default();
    slint::platform::set_platform(Box::new(Boundary(windows.clone(), buffer))).unwrap();
    let window = midnight_ui::MainWindow::new().unwrap();
    window.set_preferences_ready(true);
    midnight_ui::install_file_keys(&window);
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
    (ui, windows)
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
