//! Reveal popup controls through actual wheel input before pointer activation.
use crate::ui;
use i_slint_core::{
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
};
use slint::{ComponentHandle, LogicalPosition, platform::WindowEvent};

fn settle(test: &mut ui::TestUi) {
    for _ in 0..3 {
        test.draw();
    }
}

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (
        origin.x,
        origin.y,
        origin.x + rect.width(),
        origin.y + rect.height(),
    )
}

pub fn click(test: &mut ui::TestUi, label: &str, role: AccessibleRole) {
    let target = ui::popup_item(&test.window, label, role);
    let mut ancestors = Vec::new();
    let mut parent = target.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(item) = parent {
        if item.downcast::<Flickable>().is_some() {
            ancestors.push(item.clone());
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    // Compact Options can scroll the heading out of view while editing.
    // Reveal through the real wheel path before sending a pointer click.
    for viewport in ancestors.iter().rev() {
        let clip = bounds(viewport);
        let area = bounds(&target);
        let flick = viewport.downcast::<Flickable>().unwrap();
        // Only horizontal overflow can put a scrollbar below the control.
        // Otherwise a needless inner wheel could bubble to the outer form.
        let bottom_overlay =
            if flick.as_pin_ref().content_width().get() > viewport.geometry().width() {
                14.0
            } else {
                0.0
            };
        let delta = if area.1 < clip.1 {
            clip.1 - area.1
        } else if area.3 > clip.3 - bottom_overlay {
            clip.3 - bottom_overlay - area.3
        } else {
            0.0
        };
        if delta != 0.0 {
            test.window
                .window()
                .dispatch_event(WindowEvent::PointerScrolled {
                    position: LogicalPosition::new(clip.0 + 4.0, clip.1 + 4.0),
                    delta_x: 0.0,
                    delta_y: delta,
                });
            settle(test);
        }
    }
    let area = bounds(&target);
    for viewport in &ancestors {
        let clip = bounds(viewport);
        let overlay = if viewport
            .downcast::<Flickable>()
            .unwrap()
            .as_pin_ref()
            .content_height()
            .get()
            > viewport.geometry().height()
        {
            14.0
        } else {
            0.0
        };
        assert!(
            area.0 >= clip.0 - 1.0
                && area.1 >= clip.1 - 1.0
                && area.2 <= clip.2 + 1.0
                && area.3 <= clip.3 + 1.0,
            "{label} must be visible through its actual clip before clicking: {area:?}, {clip:?}"
        );
        assert!(
            (area.0 + area.2) / 2.0 < clip.2 - overlay,
            "{label}'s click target must not hit Fluent's overlaid vertical scrollbar"
        );
    }
    ui::click_popup(&test.window, label, role);
    settle(test);
}
