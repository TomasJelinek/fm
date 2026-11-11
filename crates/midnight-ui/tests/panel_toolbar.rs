//! Real toolbar events across view modes; sorting must not depend on visible headers.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::{LocationRole, NavigationChoiceRow};
use slint::{ComponentHandle, VecModel};
use std::{cell::RefCell, rc::Rc};

#[test]
fn each_panel_owns_its_filter_next_to_its_view_picker() {
    let mut ui = ui::TestUi::new();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let capture = calls.clone();
    ui.window
        .on_open_panel_filter(move |pane| capture.borrow_mut().push(pane));
    ui.window.set_active_pane(1);
    ui.window.set_left_filter_label("*.rs".into());
    ui.draw();
    for (pane, label, view) in [
        (0, "Left panel filter", "Left panel view mode"),
        (1, "Right panel filter", "Right panel view mode"),
    ] {
        let button = ui::root_item(&ui.window, label, AccessibleRole::Button);
        let picker = ui::root_item(&ui.window, view, AccessibleRole::Combobox);
        let button_rect = button.geometry();
        let picker_rect = picker.geometry();
        let button_origin = button.map_to_native_window(button_rect.origin);
        let picker_origin = picker.map_to_native_window(picker_rect.origin);
        assert!(button_origin.x + button_rect.width() <= picker_origin.x + 0.5);
        assert!((button_origin.y - picker_origin.y).abs() <= 1.0);
        drop((button, picker));
        ui::click_root(&ui.window, label, AccessibleRole::Button);
        assert_eq!(calls.borrow().last(), Some(&pane));
    }
}

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
    render::snapshot(&ui, "toolbar-hot-paths-centered");
    let star = ui::root_item(
        &ui.window,
        "Add current path to Hot Paths",
        AccessibleRole::Button,
    );
    let arrow = ui::root_item(&ui.window, "Hot Paths", AccessibleRole::Combobox);
    let star_rect = star.geometry();
    let arrow_rect = arrow.geometry();
    let star_origin = star.map_to_native_window(star_rect.origin);
    let arrow_origin = arrow.map_to_native_window(arrow_rect.origin);
    assert!(
        star_origin.x + star_rect.width() <= arrow_origin.x + 0.5,
        "the star and dropdown must have disjoint hit targets"
    );
    ui.window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(
                arrow_origin.x + arrow_rect.width() / 2.0,
                arrow_origin.y + arrow_rect.height() / 2.0,
            ),
        });
    ui.draw();
    render::snapshot(&ui, "toolbar-hot-paths-hover");
    drop((star, arrow));
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
