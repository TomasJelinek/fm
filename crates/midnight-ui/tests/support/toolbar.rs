//! Reveal shared toolbar controls through its actual overflow buttons.
use super::ui;
use i_slint_core::items::AccessibleRole;

pub fn reveal(test: &mut ui::TestUi, label: &str, role: AccessibleRole) {
    for _ in 0..12 {
        test.draw();
        let item = ui::root_item(&test.window, label, role);
        let bounds = item.geometry();
        let origin = item.map_to_native_window(bounds.origin);
        let left = ui::root_item(&test.window, "Scroll toolbar left", AccessibleRole::Button);
        let right = ui::root_item(&test.window, "Scroll toolbar right", AccessibleRole::Button);
        let left_edge = if left.is_visible() {
            let rect = left.geometry();
            left.map_to_native_window(rect.origin).x + rect.width() + 2.0
        } else {
            0.0
        };
        let right_edge = if right.is_visible() {
            right.map_to_native_window(right.geometry().origin).x - 2.0
        } else {
            test.width as f32
        };
        if origin.x >= left_edge - 0.5 && origin.x + bounds.width() <= right_edge + 0.5 {
            return;
        }
        ui::click_root(
            &test.window,
            if origin.x < left_edge {
                "Scroll toolbar left"
            } else {
                "Scroll toolbar right"
            },
            AccessibleRole::Button,
        );
    }
    panic!("toolbar control stayed outside its viewport: {label}");
}
