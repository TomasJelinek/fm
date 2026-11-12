//! Direct reusable operation progress fixture; adapter lifecycle has its own tests.
use crate::modeless;
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_ui::JobProgressWindow;
use slint::{
    ComponentHandle, SharedString,
    platform::{WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc};
pub struct TestUi {
    pub window: JobProgressWindow,
    native: Rc<MinimalSoftwareWindow>,
    width: usize,
    height: usize,
}
impl TestUi {
    pub fn new() -> Self {
        let adapters = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(adapters.clone()))).unwrap();
        let window = JobProgressWindow::new().unwrap();
        let weak = window.as_weak();
        window.on_request_dismiss(move || {
            if let Some(w) = weak.upgrade() {
                w.hide().unwrap();
            }
        });
        window.show().unwrap();
        let native = adapters.borrow()[0].clone();
        Self {
            window,
            native,
            width: 1340,
            height: 820,
        }
    }
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.draw();
    }
    pub fn draw(&mut self) {
        slint::platform::update_timers_and_animations();
        modeless::draw(
            &self.native,
            self.width as u32,
            self.height as u32,
            "jobs-form",
        );
    }
    pub fn snapshot(&self, name: &str) {
        modeless::draw(&self.native, self.width as u32, self.height as u32, name);
    }
}
fn find(item: ItemRc, role: AccessibleRole, label: &str) -> Option<ItemRc> {
    if item.accessible_role() == role
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|v| v == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find(item, role, label) {
            return Some(found);
        }
    }
    None
}
pub fn popup_item(window: &JobProgressWindow, label: &str, role: AccessibleRole) -> ItemRc {
    find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        role,
        label,
    )
    .unwrap_or_else(|| panic!("missing operation progress control {label}"))
}
#[allow(dead_code)] // The compact reflow fixture uses keyboard/scroll input only.
pub fn click_popup(window: &JobProgressWindow, label: &str, role: AccessibleRole) {
    let item = popup_item(window, label, role);
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    );
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: slint::platform::PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: slint::platform::PointerEventButton::Left,
        });
}
pub fn has_popup(window: &JobProgressWindow) -> bool {
    window.window().is_visible()
}
pub fn key(window: &JobProgressWindow, text: impl Into<SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
