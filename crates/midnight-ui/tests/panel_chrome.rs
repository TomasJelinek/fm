//! Header spacing and overflow retain the displayed suffix and native breadcrumb identities.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{item_tree::ItemRc, items::AccessibleRole};
use midnight_ui::PathPartRow;
use slint::{
    ComponentHandle, ModelRc, VecModel,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.size.width, rect.size.height)
}

fn settle(test: &mut ui::TestUi) {
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        test.draw();
    }
}

#[test]
fn shared_toolbar_precedes_separate_drive_and_path_rows_without_large_gaps() {
    let mut test = ui::TestUi::new();
    for density in [0, 1] {
        for font in [11, 32] {
            test.window.invoke_apply_preferences(0, density);
            test.window.invoke_apply_fonts("".into(), font, font);
            settle(&mut test);
            let (_, top_y, _, top_height) = bounds(&ui::root_item(
                &test.window,
                "Checksums",
                AccessibleRole::Button,
            ));
            let (_, drive_y, _, drive_height) = bounds(&ui::root_item(
                &test.window,
                "Drive or volume",
                AccessibleRole::Combobox,
            ));
            let upper = drive_y - top_y - top_height;
            let lower = test.window.get_left_path_top() - drive_y - drive_height;
            assert!(
                (0.0..=24.0).contains(&upper) && (0.0..=8.5).contains(&lower),
                "density={density} font={font}: toolbar→drive={upper}, drive→path={lower}"
            );
        }
    }
}

#[test]
fn overflowing_path_keeps_suffix_actions_and_full_editor_value_after_resize() {
    let mut test = ui::TestUi::new();
    let prefix = format!("C:\\{}\\", "long ancestor 文档 ".repeat(16));
    let parent = format!("{prefix}parent\\");
    let path = format!("{parent}final-😀");
    test.window.set_left_path(path.clone().into());
    test.window
        .set_left_path_parts(ModelRc::new(VecModel::from(vec![
            PathPartRow {
                identity: "raw-root".into(),
                label: prefix.clone().into(),
                prefix: prefix.into(),
            },
            PathPartRow {
                identity: "raw-parent".into(),
                label: "parent\\".into(),
                prefix: parent.clone().into(),
            },
            PathPartRow {
                identity: "raw-leaf".into(),
                label: "final-😀".into(),
                prefix: path.clone().into(),
            },
        ])));
    let opened = Rc::new(RefCell::new(Vec::new()));
    let captured = opened.clone();
    test.window
        .on_path_part(move |pane, id| captured.borrow_mut().push((pane, id)));
    let copied = Rc::new(RefCell::new(Vec::new()));
    let captured = copied.clone();
    test.window
        .on_path_copy(move |pane, id| captured.borrow_mut().push((pane, id)));
    for (width, density, font) in [(1340, 0, 11), (1040, 1, 32), (1340, 0, 11)] {
        test.resize(width, 900);
        test.window.invoke_apply_preferences(0, density);
        test.window.invoke_apply_fonts("".into(), font, font);
        settle(&mut test);
        let leaf = ui::root_item(
            &test.window,
            &format!("Open {path}"),
            AccessibleRole::Button,
        );
        let (x, y, width, height) = bounds(&leaf);
        let left = test.window.get_left_path_left();
        let right = left + test.window.get_left_path_width();
        assert!(
            x >= left && x + width <= right,
            "last path component must be visible: {x}..{} in {left}..{right}",
            x + width
        );
        ui::click(&test.window, x + width / 2.0, y + height / 2.0);
        assert_eq!(opened.borrow().last(), Some(&(0, "raw-leaf".into())));
        let ancestor = ui::root_item(
            &test.window,
            &format!("Open {parent}"),
            AccessibleRole::Button,
        );
        let (x, y, width, height) = bounds(&ancestor);
        let position = slint::LogicalPosition::new(x + width / 2.0, y + height / 2.0);
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Right,
            });
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Right,
            });
        assert_eq!(copied.borrow().last(), Some(&(0, "raw-parent".into())));
        render::snapshot(&test, &format!("path-overflow-{density}-{font}"));
    }
    ui::click_root(
        &test.window,
        "Edit full folder path",
        AccessibleRole::Button,
    );
    assert_eq!(test.window.get_left_path_draft(), path);
    ui::key(&test.window, Key::Escape);
    settle(&mut test);
    assert_eq!(test.window.get_left_path_draft(), path);
}
