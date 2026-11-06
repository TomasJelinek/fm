use midnight_viewer_ui::ViewerWindow;
use slint::{
    ComponentHandle,
    platform::{
        Key, Platform, PlatformError, WindowAdapter, WindowEvent,
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

fn key(window: &ViewerWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

#[test]
fn actual_search_widget_keeps_spaces_and_exposes_escape_to_host() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ViewerWindow::new().unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let received = events.clone();
    window.on_input_key(move |event, search| {
        received.borrow_mut().push((event.text.to_string(), search));
        event.text == slint::SharedString::from(Key::Escape) || (!search && event.text == " ")
    });
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
    window.invoke_focus_content();
    key(&window, " ");
    assert_eq!(events.borrow().last(), Some(&(" ".into(), false)));
    window.invoke_focus_search();
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    for text in ["hello", " ", "世界"] {
        key(&window, text);
    }
    assert_eq!(window.get_query(), "hello 世界");
    key(&window, Key::Escape);
    assert_eq!(
        events.borrow().last(),
        Some(&(slint::SharedString::from(Key::Escape).to_string(), true))
    );
    assert!(window.get_content_width() > 800.0 && window.get_content_height() > 450.0);
    window.hide().unwrap();
}
