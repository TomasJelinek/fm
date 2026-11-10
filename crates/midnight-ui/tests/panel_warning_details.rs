//! Warning controls must expose diagnostics without triggering row actions.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_ui::FileRow;
use slint::{
    ComponentHandle, Model, ModelRc, VecModel,
    platform::{PointerEventButton, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

#[test]
fn size_warning_is_an_accessible_detail_button_with_the_actual_reason_in_every_view() {
    let mut test = ui::TestUi::new();
    let reason = "Access denied while listing C:\\private\\資料. Click for full details.";
    test.window
        .set_left_rows(ModelRc::new(VecModel::from(vec![FileRow {
            identity: "warning:17".into(),
            name: "Folder 文档".into(),
            directory: true,
            size_state: 3,
            size_hint: reason.into(),
            ..Default::default()
        }])));
    for mode in 0..3 {
        test.window.set_left_view_mode(mode);
        test.draw();
        let warning = ui::root_item(&test.window, "Folder size warning", AccessibleRole::Button);
        assert_eq!(
            warning
                .accessible_string_property(AccessibleStringProperty::Description)
                .map(|value| value.to_string()),
            Some(reason.to_owned()),
        );
    }
}

#[test]
fn diagnostic_clicks_in_both_panels_do_not_select_open_drag_or_retry() {
    let mut test = ui::TestUi::new();
    let inspected = Rc::new(RefCell::new(Vec::new()));
    let copy = inspected.clone();
    test.window
        .on_panel_warning_details(move |pane, identity, tree| {
            copy.borrow_mut().push((pane, identity.to_string(), tree));
        });
    let leaked = Rc::new(RefCell::new(Vec::new()));
    let copy = leaked.clone();
    test.window
        .on_row_click(move |_, _, _, _| copy.borrow_mut().push("select"));
    let copy = leaked.clone();
    test.window
        .on_open(move |_, _| copy.borrow_mut().push("open"));
    let copy = leaked.clone();
    test.window
        .on_tree_disclosure(move |_, _| copy.borrow_mut().push("retry"));
    let copy = leaked.clone();
    test.window.on_row_drag(move |_, _, _, _, _| {
        copy.borrow_mut().push("drag");
        true
    });
    for pane in 0..2 {
        for mode in 0..3 {
            for tree in [false, true] {
                let rows = Rc::new(VecModel::from(vec![FileRow {
                    identity: "exact:17".into(),
                    name: "Unavailable folder".into(),
                    directory: true,
                    size_state: if tree { 0 } else { 3 },
                    size_hint: "Access denied in private child".into(),
                    tree_disclosure: if tree { 6 } else { 0 },
                    tree_error: "Permission denied while reading folder".into(),
                    ..Default::default()
                }]));
                test.window.set_left_rows(Default::default());
                test.window.set_right_rows(Default::default());
                if pane == 0 {
                    test.window.set_left_rows(rows.clone().into());
                } else {
                    test.window.set_right_rows(rows.clone().into());
                }
                test.window.set_left_view_mode(mode);
                test.window.set_right_view_mode(mode);
                test.draw();
                let label = if tree {
                    "Folder listing error"
                } else {
                    "Folder size warning"
                };
                let point = ui::root_position(&test.window, label, AccessibleRole::Button);
                inspected.borrow_mut().clear();
                ui::click(&test.window, point.x, point.y);
                assert_eq!(
                    *inspected.borrow(),
                    vec![(pane, "exact:17".to_owned(), tree)]
                );
                assert!(
                    leaked.borrow().is_empty(),
                    "pane={pane} mode={mode} tree={tree}"
                );
                ui::key(&test.window, slint::platform::Key::Return);
                assert_eq!(
                    inspected.borrow().len(),
                    2,
                    "focused warning supports Enter"
                );
                assert!(leaked.borrow().is_empty());

                // A virtualized row rebound between press and release must not inspect its replacement.
                inspected.borrow_mut().clear();
                test.window
                    .window()
                    .dispatch_event(WindowEvent::PointerPressed {
                        position: point,
                        button: PointerEventButton::Left,
                    });
                let mut row = rows.row_data(0).unwrap();
                row.identity = "replacement:18".into();
                rows.set_row_data(0, row);
                test.draw();
                test.window
                    .window()
                    .dispatch_event(WindowEvent::PointerReleased {
                        position: point,
                        button: PointerEventButton::Left,
                    });
                assert!(inspected.borrow().is_empty());
                assert!(leaked.borrow().is_empty());
                if !tree {
                    // Identical text from a new scan must not retarget a held gesture.
                    test.window
                        .window()
                        .dispatch_event(WindowEvent::PointerPressed {
                            position: point,
                            button: PointerEventButton::Left,
                        });
                    let mut row = rows.row_data(0).unwrap();
                    row.warning_revision = "new scan, identical reason".into();
                    rows.set_row_data(0, row);
                    test.draw();
                    test.window
                        .window()
                        .dispatch_event(WindowEvent::PointerReleased {
                            position: point,
                            button: PointerEventButton::Left,
                        });
                    assert!(inspected.borrow().is_empty());
                    assert!(leaked.borrow().is_empty());
                }
            }
        }
    }
}

#[test]
fn panel_notice_glyph_opens_the_complete_description_without_copying_or_dismissing() {
    let mut test = ui::TestUi::new();
    let text = format!(
        "Access denied\nPath\nC:\\資料\\{}\\private",
        "long".repeat(150)
    );
    test.window.set_left_error(text.as_str().into());
    let inspected = Rc::new(RefCell::new(Vec::new()));
    let copy = inspected.clone();
    test.window
        .on_panel_error_details(move |_, text| copy.borrow_mut().push(text.to_string()));
    test.draw();
    ui::click_root(&test.window, "Show error details", AccessibleRole::Button);
    assert_eq!(*inspected.borrow(), vec![text.clone()]);
    assert_eq!(test.window.get_left_error().as_str(), text);
}

#[test]
fn same_text_from_a_new_error_cannot_retarget_held_detail_or_copy_gestures() {
    let mut test = ui::TestUi::new();
    test.window
        .set_left_error("Same bounded description".into());
    test.window
        .set_left_error_revision("first occurrence".into());
    let actions = Rc::new(RefCell::new(Vec::new()));
    let captured = actions.clone();
    test.window
        .on_panel_error_details(move |_, _| captured.borrow_mut().push("details"));
    let captured = actions.clone();
    test.window
        .on_copy_panel_error(move |_, _| captured.borrow_mut().push("copy"));
    for label in ["Show error details", "Copy error details"] {
        test.draw();
        let position = ui::root_position(&test.window, label, AccessibleRole::Button);
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
        test.window
            .set_left_error_revision(format!("replacement for {label}").into());
        test.draw();
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        assert!(
            actions.borrow().is_empty(),
            "{label} acted on a replacement error"
        );
    }
}
