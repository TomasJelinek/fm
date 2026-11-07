use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, window::WindowInner,
};
use midnight_viewer_ui::ViewerWindow;
use slint::{
    ComponentHandle,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, rc::Rc};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn find_control(item: ItemRc, label: &str) -> Option<ItemRc> {
    if item
        .accessible_string_property(AccessibleStringProperty::Label)
        .is_some_and(|value| value == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find_control(item, label) {
            return Some(found);
        }
    }
    None
}

#[test]
fn document_pager_disables_endpoints_and_sends_relative_page_requests() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_image_mode(true);
    window.set_document_page(1);
    window.set_document_pages(2);
    let turns = Rc::new(RefCell::new(Vec::new()));
    let received = turns.clone();
    window.on_document_turn(move |delta| received.borrow_mut().push(delta));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(900, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(900.0, 600.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 600];
    let mut click = |label: &str| {
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 900);
        });
        let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
        let item = find_control(root, label).unwrap();
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        let position = slint::LogicalPosition::new(
            origin.x + rect.size.width / 2.0,
            origin.y + rect.size.height / 2.0,
        );
        let button = PointerEventButton::Left;
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        window
            .window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        window
            .window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
    };
    click("Previous document page (Ctrl+PageUp)");
    assert!(turns.borrow().is_empty());
    click("Next document page (Ctrl+PageDown)");
    assert_eq!(*turns.borrow(), [1]);
    window.set_document_page(2);
    click("Next document page (Ctrl+PageDown)");
    assert_eq!(*turns.borrow(), [1]);
    click("Previous document page (Ctrl+PageUp)");
    assert_eq!(*turns.borrow(), [1, -1]);
    assert!(window.get_content_focused());
    window.hide().unwrap();
}

#[test]
fn wrap_checkbox_emits_changes_only_when_available_and_returns_content_focus() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    let changes = Rc::new(RefCell::new(Vec::new()));
    let received = changes.clone();
    window.on_change_wrap(move |wrap| received.borrow_mut().push(wrap));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(900, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(900.0, 600.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 600];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    let root = WindowInner::from_pub(window.window()).component();
    let item = find_control(ItemRc::new_root(root), "Word wrap").expect("accessible Wrap checkbox");
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    );
    let click = || {
        let button = PointerEventButton::Left;
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        window
            .window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        window
            .window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
    };
    click();
    assert!(
        changes.borrow().is_empty(),
        "Binary/Markdown cannot toggle wrapping"
    );
    window.set_wrap_available(true);
    click();
    assert_eq!(*changes.borrow(), [true]);
    assert!(window.get_content_focused());
    window.set_wrap_enabled(true);
    click();
    assert_eq!(*changes.borrow(), [true, false]);
    window.hide().unwrap();
}

#[test]
fn image_controls_hide_text_tools_and_route_buttons_drag_and_control_wheel() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    window.set_image_mode(true);
    window.set_search_visible(true);
    let commands = Rc::new(RefCell::new(Vec::new()));
    let received = commands.clone();
    window.on_image_command(move |command| received.borrow_mut().push(command));
    let zoom = Rc::new(RefCell::new(Vec::new()));
    let received = zoom.clone();
    window.on_image_zoom(move |delta, x, y| received.borrow_mut().push((delta, x, y)));
    let drag = Rc::new(RefCell::new(Vec::new()));
    let received = drag.clone();
    window.on_image_drag(move |x, y| received.borrow_mut().push((x, y)));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(900, 600));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(900.0, 600.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 600];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    assert!(find_control(root.clone(), "Word wrap").is_none());
    assert!(find_control(root.clone(), "Viewer mode").is_none());
    assert!(
        window.get_content_height() > 540.0,
        "hidden search has no layout height"
    );
    let button = PointerEventButton::Left;
    for label in [
        "Fit image to window",
        "Actual image pixels",
        "Zoom image in",
        "Zoom image out",
    ] {
        let item = find_control(root.clone(), label).unwrap();
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        let position = slint::LogicalPosition::new(
            origin.x + rect.size.width / 2.0,
            origin.y + rect.size.height / 2.0,
        );
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        window
            .window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        window
            .window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
    }
    assert_eq!(*commands.borrow(), [0, 1, 2, 3]);
    assert!(window.get_content_focused());
    let position = slint::LogicalPosition::new(250.0, 150.0);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: slint::platform::Key::Control.into(),
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: 120.0,
        });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: slint::platform::Key::Control.into(),
    });
    assert_eq!(zoom.borrow().len(), 1);
    assert_eq!(zoom.borrow()[0].0, 120.0);
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    let position = slint::LogicalPosition::new(270.0, 180.0);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
    assert_eq!(*drag.borrow(), [(20.0, 30.0)]);
    window.hide().unwrap();
}
