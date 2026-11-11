//! Input for the actual native filter child in Engine-backed tree fixtures.
#[path = "window_input.rs"]
mod input;

use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use slint::{
    ComponentHandle,
    platform::{WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::rc::Rc;

pub use input::{chord, key};

pub fn filter(h: &super::support::Harness) -> Rc<MinimalSoftwareWindow> {
    assert!(h.ui.window.get_panel_filter_view().open);
    let child = h.dialog();
    assert!(
        WindowInner::from_pub(h.ui.window.window())
            .active_popups()
            .is_empty(),
        "the filter must not use an embedded owner popup"
    );
    control(&child, "File filter expression", AccessibleRole::TextInput);
    child
}

fn find(item: ItemRc, label: &str, role: AccessibleRole) -> Option<ItemRc> {
    if item.accessible_role() == role
        && item.is_visible()
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find(item, label, role) {
            return Some(found);
        }
    }
    None
}

fn control(window: &MinimalSoftwareWindow, label: &str, role: AccessibleRole) -> ItemRc {
    let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    find(root, label, role).unwrap_or_else(|| panic!("Missing visible filter control: {label}"))
}

pub fn click(window: &MinimalSoftwareWindow, label: &str, role: AccessibleRole) {
    input::click(window, &control(window, label, role));
}

#[allow(dead_code)] // Only tree_filters exercises the minimum-size Cancel control.
pub fn resize(window: &MinimalSoftwareWindow, width: u32, height: u32) {
    window.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); width as usize * height as usize];
    window.window().request_redraw();
    assert!(window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    }));
}
