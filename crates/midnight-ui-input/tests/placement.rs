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

slint::slint! {
    export component TestWindow inherits Window {
        min-width: 1px; min-height: 1px;
        out property <length> content-width: self.width;
        out property <length> content-height: self.height;
    }
}

struct RecordingWindow {
    window: Window,
    renderer: SoftwareRenderer,
    position: Cell<PhysicalPosition>,
    size: Cell<PhysicalSize>,
    resize_scale: Cell<f32>,
}
impl RecordingWindow {
    fn new() -> Rc<Self> {
        Rc::new_cyclic(|weak: &Weak<Self>| Self {
            window: Window::new(weak.clone()),
            renderer: SoftwareRenderer::new(),
            position: Cell::new(PhysicalPosition::new(0, 0)),
            size: Cell::new(PhysicalSize::new(1, 1)),
            resize_scale: Cell::new(0.0),
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
        self.resize_scale.set(self.window.scale_factor());
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

#[test]
fn captured_parent_dpi_is_used_for_the_childs_first_layout_before_native_creation() {
    let recorded = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(TestPlatform(recorded.clone()))).unwrap();
    let parent = TestWindow::new().unwrap();
    for scale in [1.25, 1.5, 2.25] {
        parent
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: scale,
            });
        parent.window().set_size(PhysicalSize::new(2250, 1575));
        let placement = WindowPlacement::capture(parent.window());
        let child = TestWindow::new().unwrap();
        assert_eq!(child.window().scale_factor(), 1.0);
        placement.apply(child.window());
        assert_eq!(
            child.window().scale_factor(),
            scale,
            "first backend font scale must match the parent's monitor"
        );
        assert_eq!(child.window().size(), parent.window().size());
        assert_eq!(child.get_content_width(), placement.logical_size().width);
        assert_eq!(child.get_content_height(), placement.logical_size().height);
        assert_eq!(
            recorded.borrow().last().unwrap().resize_scale.get(),
            scale,
            "captured DPI must be applied before physical sizing establishes logical layout"
        );
        assert!(!child.window().is_visible());
        child
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.75 });
        assert_eq!(
            child.window().scale_factor(),
            1.75,
            "native DPI remains authoritative after creation"
        );
    }
}

#[test]
fn logical_only_placement_keeps_the_childs_existing_scale() {
    let recorded = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(TestPlatform(recorded))).unwrap();
    let child = TestWindow::new().unwrap();
    child
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    WindowPlacement::from(slint::LogicalSize::new(640.0, 400.0)).apply(child.window());
    assert_eq!(child.window().scale_factor(), 1.5);
    assert_eq!(child.window().size(), PhysicalSize::new(960, 600));
    assert_eq!(child.get_content_width(), 640.0);
    assert_eq!(child.get_content_height(), 400.0);
}
