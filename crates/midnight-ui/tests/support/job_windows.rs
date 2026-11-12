//! Distinct software-native adapters for installed Jobs and decision hosts.
#[path = "window_input.rs"]
mod input;
#[allow(dead_code)]
#[path = "modeless.rs"]
mod modeless;
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_ui::MainWindow;
use slint::{
    ComponentHandle, SharedString,
    platform::{WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc};
pub struct TestUi {
    pub window: MainWindow,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    width: u32,
    height: u32,
}
impl TestUi {
    pub fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let window = MainWindow::new().unwrap();
        midnight_ui::install_file_keys(&window);
        window.set_preferences_ready(true);
        window.show().unwrap();
        window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        modeless::draw(&windows.borrow()[0], 1000, 700, "jobs-errors-owner");
        Self {
            window,
            windows,
            width: 700,
            height: 500,
        }
    }
    pub fn draw(&self) {
        slint::platform::update_timers_and_animations();
        let children: Vec<_> = self
            .windows
            .borrow()
            .iter()
            .skip(1)
            .filter(|w| w.window().is_visible())
            .cloned()
            .collect();
        for child in children {
            modeless::draw(&child, self.width, self.height, "jobs-errors-child");
        }
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.draw();
    }
    pub fn child(&self) -> Rc<MinimalSoftwareWindow> {
        self.windows
            .borrow()
            .iter()
            .skip(1)
            .rev()
            .find(|w| w.window().is_visible())
            .cloned()
            .expect("visible native Jobs/decision child")
    }
    pub fn visible_count(&self) -> usize {
        self.windows
            .borrow()
            .iter()
            .skip(1)
            .filter(|w| w.window().is_visible())
            .count()
    }
    pub fn item(&self, label: &str, role: AccessibleRole) -> ItemRc {
        fn find(item: ItemRc, role: AccessibleRole, label: &str) -> Option<ItemRc> {
            if item.is_visible()
                && item.accessible_role() == role
                && item
                    .accessible_string_property(AccessibleStringProperty::Label)
                    .is_some_and(|value| value == label)
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
        let child = self.child();
        find(
            ItemRc::new_root(WindowInner::from_pub(child.window()).component()),
            role,
            label,
        )
        .unwrap_or_else(|| panic!("missing visible native control {label}"))
    }
    pub fn click(&self, label: &str) {
        input::click(&self.child(), &self.item(label, AccessibleRole::Button));
        self.draw();
    }
    pub fn key(&self, text: impl Into<SharedString>) {
        input::key(&self.child(), text);
        self.draw();
    }
    pub fn snapshot(&self, name: &str) {
        modeless::draw(&self.child(), self.width, self.height, name);
    }
    pub fn click_owner(&self, x: f32, y: f32) {
        let position = slint::LogicalPosition::new(x, y);
        for event in [
            WindowEvent::PointerPressed {
                position,
                button: slint::platform::PointerEventButton::Left,
            },
            WindowEvent::PointerReleased {
                position,
                button: slint::platform::PointerEventButton::Left,
            },
        ] {
            self.window.window().dispatch_event(event);
        }
    }
}
