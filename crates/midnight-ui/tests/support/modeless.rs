//! Shared software-native-window harness for independent tool windows.
#[path = "bitmap.rs"]
pub mod bitmap;
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use slint::platform::{
    Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{cell::RefCell, rc::Rc};

pub struct Windows(pub Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>);
impl Platform for Windows {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.0.borrow_mut().push(window.clone());
        Ok(window)
    }
}
pub fn draw(window: &MinimalSoftwareWindow, width: u32, height: u32, name: &str) {
    window.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); width as usize * height as usize];
    window.window().request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    });
    bitmap::snapshot(
        &pixels,
        width as usize,
        height as usize,
        &format!("{name}-{width}x{height}"),
    );
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
        if let Some(item) = find(item, role, label) {
            return Some(item);
        }
    }
    None
}
pub fn accessible_item(
    window: &MinimalSoftwareWindow,
    role: AccessibleRole,
    label: &str,
) -> Option<ItemRc> {
    let root = WindowInner::from_pub(window.window()).component();
    find(ItemRc::new_root(root), role, label)
}
#[allow(dead_code)] // The Find and checksum harnesses share this module.
pub fn accessible_popup_item(
    window: &MinimalSoftwareWindow,
    role: AccessibleRole,
    label: &str,
) -> Option<ItemRc> {
    let inner = WindowInner::from_pub(window.window());
    inner
        .active_popups()
        .iter()
        .rev()
        .find_map(|popup| find(ItemRc::new_root(popup.component.clone()), role, label))
}
pub fn click(window: &MinimalSoftwareWindow, label: &str) {
    let item = accessible_popup_item(window, AccessibleRole::Button, label)
        .or_else(|| accessible_item(window, AccessibleRole::Button, label))
        .unwrap_or_else(|| panic!("Missing {label}"));
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    );
    assert!(position.x >= 0.0 && position.y >= 0.0);
    assert!(
        position.x < window.window().size().width as f32
            && position.y < window.window().size().height as f32,
        "{label} is outside the window: {position:?}"
    );
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}
