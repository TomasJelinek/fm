use midnight_ui_input::WindowPlacement;
use slint::{
    ComponentHandle, PhysicalPosition, PhysicalSize, Window, WindowPosition, WindowSize,
    platform::{
        Platform, PlatformError, Renderer, WindowAdapter, WindowEvent,
        software_renderer::SoftwareRenderer,
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

slint::slint! { export component TestWindow inherits Window { min-width: 1px; min-height: 1px; } }

struct RecordingWindow {
    window: Window,
    renderer: SoftwareRenderer,
    position: Cell<PhysicalPosition>,
    size: Cell<PhysicalSize>,
}
impl RecordingWindow {
    fn new() -> Rc<Self> {
        Rc::new_cyclic(|weak: &Weak<Self>| Self {
            window: Window::new(weak.clone()),
            renderer: SoftwareRenderer::new(),
            position: Cell::new(PhysicalPosition::new(0, 0)),
            size: Cell::new(PhysicalSize::new(1, 1)),
        })
    }
}
impl WindowAdapter for RecordingWindow {
    fn window(&self) -> &Window {
        &self.window
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
    fn position(&self) -> Option<PhysicalPosition> {
        Some(self.position.get())
    }
    fn set_position(&self, position: WindowPosition) {
        self.position
            .set(position.to_physical(self.window.scale_factor()));
    }
    fn size(&self) -> PhysicalSize {
        self.size.get()
    }
    fn set_size(&self, size: WindowSize) {
        self.size.set(size.to_physical(self.window.scale_factor()));
        self.window.dispatch_event(WindowEvent::Resized {
            size: size.to_logical(self.window.scale_factor()),
        });
    }
}
struct TestPlatform(Rc<RefCell<Vec<Rc<RecordingWindow>>>>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = RecordingWindow::new();
        self.0.borrow_mut().push(window.clone());
        Ok(window)
    }
}

#[test]
fn child_uses_parent_physical_bounds_before_show_across_dpi_and_native_states() {
    let recorded = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(TestPlatform(recorded.clone()))).unwrap();
    let parent = TestWindow::new().unwrap();
    parent
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    parent
        .window()
        .set_position(PhysicalPosition::new(-1820, 135));
    parent.window().set_size(PhysicalSize::new(960, 600));
    for (maximized, fullscreen) in [(false, false), (true, false), (false, true)] {
        parent.window().set_maximized(maximized);
        parent.window().set_fullscreen(fullscreen);
        let placement = WindowPlacement::capture(parent.window());
        assert_eq!(
            placement.logical_size(),
            slint::LogicalSize::new(640.0, 400.0)
        );
        let child = TestWindow::new().unwrap();
        child
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
        placement.apply(child.window());
        assert_eq!(child.window().position(), PhysicalPosition::new(-1820, 135));
        assert_eq!(child.window().size(), PhysicalSize::new(960, 600));
        assert_eq!(child.window().is_maximized(), maximized);
        assert_eq!(child.window().is_fullscreen(), fullscreen);
        assert!(
            !child.window().is_visible(),
            "placement is applied before showing"
        );
    }
}
