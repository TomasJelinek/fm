//! Real toolbar events across view modes; sorting must not depend on visible headers.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::{LocationRole, NavigationChoiceRow};
use slint::{ComponentHandle, VecModel};
use std::{cell::RefCell, rc::Rc};

#[test]
fn sort_and_hot_path_actions_are_available_without_detailed_headers() {
    let mut ui = ui::TestUi::new();
    let sorts = Rc::new(RefCell::new(Vec::new()));
    let capture = sorts.clone();
    ui.window
        .on_sort(move |pane, column| capture.borrow_mut().push((pane, column)));
    for mode in [1, 2, 0] {
        ui.window.set_left_view_mode(mode);
        ui.draw();
        ui::click_root(&ui.window, "Left panel sort", AccessibleRole::Combobox);
        ui.draw();
        ui::click_popup(&ui.window, "Size", AccessibleRole::Button);
        ui.draw();
        assert_eq!(sorts.borrow().last(), Some(&(0, 1)));
    }
    let added = Rc::new(RefCell::new(Vec::new()));
    let capture = added.clone();
    ui.window.on_bookmark_add(move |pane, name, slot| {
        capture.borrow_mut().push((pane, name.to_string(), slot));
    });
    ui.window.set_left_bookmark_name("文档".into());
    ui.window.set_hot_path_choices(
        Rc::new(VecModel::from(vec![
            NavigationChoiceRow {
                identity: "add-hot-path".into(),
                label: "Add path".into(),
                kind: LocationRole::HotPath,
                ..Default::default()
            },
            NavigationChoiceRow {
                identity: "manage-hot-paths".into(),
                label: "Manage Hot Paths…".into(),
                kind: LocationRole::HotPath,
                ..Default::default()
            },
        ]))
        .into(),
    );
    ui.draw();
    ui::click_root(
        &ui.window,
        "Add current path to Hot Paths",
        AccessibleRole::Button,
    );
    assert_eq!(added.borrow().as_slice(), &[(0, "文档".into(), 0)]);
    assert!(
        !ui::has_popup(&ui.window),
        "star adds directly; arrow opens menu"
    );
    ui::click_root(&ui.window, "Hot Paths", AccessibleRole::Combobox);
    ui.draw();
    ui::click_popup(&ui.window, "Add path", AccessibleRole::Button);
    assert_eq!(added.borrow().len(), 2);
    ui.window.hide().unwrap();
}
