//! Direct native operation form fixture; lifetime/identity tests use installed adapters instead.
#![allow(dead_code)]
#[path = "modeless.rs"]
mod modeless;
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_ui::OperationWindow;
use slint::{
    ComponentHandle, SharedString,
    platform::{PointerEventButton, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc};

pub struct TestUi {
    pub window: OperationWindow,
    pub native: Rc<MinimalSoftwareWindow>,
    pub pixels: Vec<slint::Rgb8Pixel>,
    pub width: usize,
    pub height: usize,
}
impl TestUi {
    pub fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let window = OperationWindow::new().unwrap();
        window.invoke_apply_preferences(0, 0, "".into(), 11, 15);
        window.show().unwrap();
        let native = windows.borrow()[0].clone();
        let mut test = Self {
            window,
            native,
            pixels: Vec::new(),
            width: 0,
            height: 0,
        };
        test.resize(640, 420);
        test
    }
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.native
            .set_size(slint::PhysicalSize::new(width as u32, height as u32));
        self.window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32, height as f32),
        });
        self.pixels
            .resize(width * height, slint::Rgb8Pixel::default());
        self.draw();
    }
    pub fn draw(&mut self) {
        slint::platform::update_timers_and_animations();
        self.window.window().request_redraw();
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut self.pixels, self.width);
        });
    }
    pub fn preferred_height(&self) -> f32 {
        let component = WindowInner::from_pub(self.window.window()).component();
        let component = i_slint_core::item_tree::ItemTreeRc::borrow_pin(&component);
        component
            .as_ref()
            .layout_info(i_slint_core::layout::Orientation::Vertical)
            .preferred_bounded()
    }
}
pub fn root_item(window: &OperationWindow, text: &str, role: AccessibleRole) -> ItemRc {
    fn find(item: ItemRc, text: &str, role: AccessibleRole) -> Option<ItemRc> {
        if item.accessible_role() == role
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|label| label == text)
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(found) = find(item, text, role) {
                return Some(found);
            }
        }
        None
    }
    find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        text,
        role,
    )
    .unwrap_or_else(|| panic!("No operation control: {text}"))
}
pub fn key(window: &OperationWindow, text: impl Into<SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
pub fn click(window: &OperationWindow, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    for event in [
        WindowEvent::PointerMoved { position },
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        window.window().dispatch_event(event);
    }
}

pub fn snapshot(test: &TestUi, name: &str) {
    modeless::bitmap::snapshot(&test.pixels, test.width, test.height, name);
}
