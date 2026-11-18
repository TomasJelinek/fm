//! The shipped embedded operation form in a bare main window, driven through
//! `OperationPresentation` as the installed adapter drives it. Lifetime and identity
//! tests use the installed adapter instead.
#![allow(dead_code)]
#[path = "embedded_form.rs"]
mod embedded_form;
#[path = "modeless.rs"]
mod modeless;
pub use embedded_form::{assert_inside, body};
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_ui::{MainWindow, OperationPresentation};
use slint::{
    ComponentHandle, SharedString,
    platform::{PointerEventButton, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc};

pub struct TestUi {
    pub main: MainWindow,
    pub native: Rc<MinimalSoftwareWindow>,
    pub pixels: Vec<slint::Rgb8Pixel>,
    pub width: usize,
    pub height: usize,
}
impl TestUi {
    /// The form's popup shows once [`Self::begin`] runs; its token never changes.
    pub fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        main.set_preferences_ready(true);
        main.show().unwrap();
        main.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        let native = windows.borrow()[0].clone();
        let mut test = Self {
            main,
            native,
            pixels: Vec::new(),
            width: 0,
            height: 0,
        };
        test.apply_preferences(0, 0, "", 11, 15);
        test.form().set_token("operation".into());
        test.resize(640, 420);
        test
    }
    pub fn form(&self) -> OperationPresentation<'_> {
        self.main.global::<OperationPresentation>()
    }
    /// (Re)start the form, as the adapter does for each presented operation.
    pub fn begin(&self) {
        self.form().invoke_begin();
        slint::platform::update_timers_and_animations();
    }
    pub fn apply_preferences(
        &self,
        scheme: i32,
        density: i32,
        family: &str,
        desktop: i32,
        touch: i32,
    ) {
        self.main.invoke_apply_fonts(family.into(), desktop, touch);
        self.main.invoke_apply_preferences(scheme, density);
    }
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.native
            .set_size(slint::PhysicalSize::new(width as u32, height as u32));
        self.main.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32, height as f32),
        });
        self.pixels
            .resize(width * height, slint::Rgb8Pixel::default());
        self.draw();
    }
    pub fn draw(&mut self) {
        slint::platform::update_timers_and_animations();
        self.main.window().request_redraw();
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut self.pixels, self.width);
        });
    }
    /// [`Self::draw`] without painting: geometry is current, pixels keep the last frame.
    pub fn layout(&mut self) {
        slint::platform::update_timers_and_animations();
        WindowInner::from_pub(self.main.window()).ensure_tree_instantiated();
    }
    /// The open form's popup root.
    pub fn popup(&self) -> ItemRc {
        let inner = WindowInner::from_pub(self.main.window());
        let popups = inner.active_popups();
        let popup = popups
            .iter()
            .rev()
            .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .expect("the embedded operation form is open");
        ItemRc::new_root(popup.component.clone())
    }
    /// The dialog height the form asks for: its body while the window is larger.
    pub fn preferred_height(&self) -> f32 {
        body(&root_item(self, "Cancel", AccessibleRole::Button))
            .1
            .height
    }
}
pub fn root_item(test: &TestUi, text: &str, role: AccessibleRole) -> ItemRc {
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
    find(test.popup(), text, role).unwrap_or_else(|| panic!("No operation control: {text}"))
}
pub fn key(test: &TestUi, text: impl Into<SharedString>) {
    let text = text.into();
    test.main
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    test.main
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
pub fn click(test: &TestUi, x: f32, y: f32) {
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
        test.main.window().dispatch_event(event);
    }
}

pub fn snapshot(test: &TestUi, name: &str) {
    modeless::bitmap::snapshot(&test.pixels, test.width, test.height, name);
}
