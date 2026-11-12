//! Real row widgets keep disclosure separate from navigation and selection.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibilityAction,
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem},
};
use midnight_ui::{FileRow, MainWindow};
use slint::{
    ComponentHandle, Model, VecModel,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

fn folders() -> Rc<VecModel<FileRow>> {
    Rc::new(VecModel::from(vec![FileRow {
        identity: "opaque occurrence:17".into(),
        name: "Folder 文档".into(),
        directory: true,
        tree_disclosure: 3,
        ..Default::default()
    }]))
}

fn pointer(window: &MainWindow, position: slint::LogicalPosition, pressed: bool) {
    let button = PointerEventButton::Left;
    window.window().dispatch_event(if pressed {
        WindowEvent::PointerPressed { position, button }
    } else {
        WindowEvent::PointerReleased { position, button }
    });
}

fn button(window: &MainWindow, label: &str) -> ItemRc {
    ui::root_item(window, label, AccessibleRole::Button)
}

fn visible_image(item: ItemRc) -> Option<ItemRc> {
    if item.is_visible() && item.downcast::<ImageItem>().is_some() {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(image) = visible_image(item) {
            return Some(image);
        }
    }
    None
}

fn assert_retry_svg(target: &ItemRc) {
    assert!(
        ui::visible_text_values(target.clone()).is_empty(),
        "retry must not depend on a font glyph"
    );
    let image = visible_image(target.clone()).expect("retry has visible image artwork");
    let source = image.downcast::<ImageItem>().unwrap();
    let actual = source
        .as_pin_ref()
        .source()
        .to_rgba8()
        .expect("retry image renders");
    let expected = slint::Image::load_from_svg_data(include_bytes!("../ui/icons/refresh.svg"))
        .unwrap()
        .to_rgba8()
        .unwrap();
    assert_eq!(actual.size(), expected.size());
    assert_eq!(
        actual.as_bytes(),
        expected.as_bytes(),
        "retry must use the refresh SVG"
    );
    assert!(actual.as_slice().iter().any(|pixel| pixel.a > 0));
    let rect = image.geometry();
    let origin = image.map_to_native_window(rect.origin);
    let target_origin = target.map_to_native_window(target.geometry().origin);
    assert!(rect.width() > 0.0 && rect.width() <= 16.0);
    assert!(rect.height() > 0.0 && rect.height() <= 16.0);
    assert!(origin.x >= target_origin.x && origin.y >= target_origin.y);
    assert!(origin.x + rect.width() <= target_origin.x + target.geometry().width());
    assert!(origin.y + rect.height() <= target_origin.y + target.geometry().height());
}

#[test]
fn disclosure_clicks_do_not_select_open_or_drag_in_any_layout_or_pane() {
    let mut ui = ui::TestUi::new();
    let rows = folders();
    let disclosed = Rc::new(RefCell::new(Vec::new()));
    let captured = disclosed.clone();
    ui.window.on_tree_disclosure(move |pane, identity| {
        captured.borrow_mut().push((pane, identity.to_string()));
    });
    let leaked = Rc::new(RefCell::new(Vec::new()));
    let captured = leaked.clone();
    ui.window
        .on_row_click(move |_, _, _, _| captured.borrow_mut().push("selection"));
    let captured = leaked.clone();
    ui.window
        .on_open(move |_, _| captured.borrow_mut().push("open"));
    let captured = leaked.clone();
    ui.window.on_row_drag(move |_, _, _, _, _| {
        captured.borrow_mut().push("drag");
        true
    });
    for pane in 0..2 {
        ui.window.set_left_rows(Default::default());
        ui.window.set_right_rows(Default::default());
        if pane == 0 {
            ui.window.set_left_rows(rows.clone().into());
        } else {
            ui.window.set_right_rows(rows.clone().into());
        }
        for mode in 0..3 {
            ui.window.set_left_view_mode(mode);
            ui.window.set_right_view_mode(mode);
            ui.draw();
            let point = ui::root_position(
                &ui.window,
                "Expand folders in Folder 文档",
                AccessibleRole::Button,
            );
            disclosed.borrow_mut().clear();
            for _ in 0..2 {
                ui::click(&ui.window, point.x, point.y);
            }
            assert_eq!(
                *disclosed.borrow(),
                vec![(pane, "opaque occurrence:17".to_owned()); 2]
            );
            assert!(leaked.borrow().is_empty(), "pane={pane}, mode={mode}");

            // A disclosure drag remains owned by the disclosure target.
            pointer(&ui.window, point, true);
            let outside = slint::LogicalPosition::new(point.x + 60.0, point.y + 2.0);
            ui.window
                .window()
                .dispatch_event(WindowEvent::PointerMoved { position: outside });
            pointer(&ui.window, outside, false);
            assert_eq!(disclosed.borrow().len(), 2);
            assert!(leaked.borrow().is_empty());
        }
    }
}

#[test]
fn disclosure_rejects_replaced_rows_changed_states_and_cancelled_presses() {
    let mut ui = ui::TestUi::new();
    let rows = folders();
    ui.window.set_left_rows(rows.clone().into());
    let calls = Rc::new(RefCell::new(Vec::new()));
    let captured = calls.clone();
    ui.window
        .on_tree_disclosure(move |_, identity| captured.borrow_mut().push(identity));
    for mode in 0..3 {
        ui.window.set_left_view_mode(mode);
        for change in 0..3 {
            rows.set_row_data(0, folders().row_data(0).unwrap());
            ui.draw();
            let point = ui::root_position(
                &ui.window,
                "Expand folders in Folder 文档",
                AccessibleRole::Button,
            );
            pointer(&ui.window, point, true);
            if change < 2 {
                let mut row = rows.row_data(0).unwrap();
                if change == 0 {
                    row.identity = "replacement occurrence".into();
                } else {
                    row.tree_disclosure = 2;
                }
                rows.set_row_data(0, row);
                ui.draw();
            } else {
                ui.window
                    .window()
                    .dispatch_event(WindowEvent::PointerExited);
            }
            pointer(&ui.window, point, false);
            assert!(calls.borrow().is_empty(), "mode={mode}, change={change}");
        }
    }
}

#[test]
fn disclosure_states_and_deep_indent_preserve_row_geometry() {
    let mut ui = ui::TestUi::new();
    let rows = folders();
    ui.window.set_left_rows(rows.clone().into());
    let retries = Rc::new(RefCell::new(Vec::new()));
    let captured = retries.clone();
    ui.window.on_tree_disclosure(move |pane, identity| {
        captured.borrow_mut().push((pane, identity.to_string()));
    });
    for mode in 0..3 {
        ui.window.set_left_view_mode(mode);
        ui.draw();
        let baseline =
            ui::root_item(&ui.window, "Folder 文档", AccessibleRole::ListItem).geometry();
        for (state, expanded, label, glyph) in [
            (1, false, "Discover folders in Folder 文档", "›"),
            (2, false, "Loading folders in Folder 文档", "…"),
            (3, false, "Expand folders in Folder 文档", "+"),
            (4, true, "Collapse folders in Folder 文档", "−"),
            (6, false, "Retry folders in Folder 文档", ""),
            (6, true, "Collapse folders in Folder 文档", "−"),
        ] {
            let mut row = rows.row_data(0).unwrap();
            row.tree_disclosure = state;
            row.tree_expanded = expanded;
            row.tree_depth = 64;
            rows.set_row_data(0, row);
            ui.draw();
            let target = button(&ui.window, label);
            if state == 6 && !expanded {
                assert_retry_svg(&target);
                let before = retries.borrow().len();
                target.accessible_action(&AccessibilityAction::Default);
                assert_eq!(retries.borrow().len(), before + 1);
                assert_eq!(
                    retries.borrow().last().unwrap(),
                    &(0, "opaque occurrence:17".into())
                );
            } else {
                assert!(
                    ui::visible_text_values(target.clone())
                        .iter()
                        .any(|text| text == glyph)
                );
            }
            let row = ui::root_item(&ui.window, "Folder 文档", AccessibleRole::ListItem);
            assert_eq!(row.geometry(), baseline, "mode={mode}, state={state}");
            let origin = target.map_to_native_window(target.geometry().origin);
            let row_origin = row.map_to_native_window(row.geometry().origin);
            assert!(
                origin.x >= row_origin.x
                    && origin.x + target.geometry().width()
                        <= row_origin.x + row.geometry().width()
            );
        }
    }
    // Modified disclosure clicks still only disclose; modifiers cannot leak to selection.
    ui.window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    ui::click_root(
        &ui.window,
        "Collapse folders in Folder 文档",
        AccessibleRole::Button,
    );
    ui.window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}
