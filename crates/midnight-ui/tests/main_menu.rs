//! F10 exposes commands through the same callbacks as toolbar and keyboard actions.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use slint::platform::Key;
use std::{cell::RefCell, rc::Rc};

#[test]
fn main_menu_exposes_file_actions_and_dispatches_to_the_active_panel() {
    let mut test = ui::TestUi::new();
    test.window.set_active_pane(1);
    test.window.set_right_can_target_cursor(true);
    test.window.set_right_can_target_selection(true);
    let actions = Rc::new(RefCell::new(Vec::new()));
    let captured = actions.clone();
    test.window.on_mutation(move |pane, operation| {
        captured.borrow_mut().push((pane, operation));
    });
    test.window.invoke_focus_files();
    ui::key(&test.window, Key::F10);
    test.draw();
    for label in [
        "View",
        "View as plain text",
        "Edit",
        "Copy to…",
        "Move to…",
        "Rename…",
        "Create folder…",
        "Create empty file…",
        "Move to Trash…",
        "Delete permanently…",
        "Select all",
        "Select files only",
        "Deselect all",
        "Invert selection",
        "Find files…",
        "Checksums…",
        "Make File List…",
        "Filter panel…",
        "Compare files…",
        "Compare directories…",
        "Options",
        "Jobs",
    ] {
        ui::popup_item(&test.window, label, AccessibleRole::Button);
    }
    render::snapshot(&test, "main-menu-files");
    ui::click_popup(&test.window, "Rename…", AccessibleRole::Button);
    test.draw();
    assert_eq!(*actions.borrow(), [(1, 0)]);
    assert!(!ui::has_popup(&test.window));
}

fn enabled(test: &ui::TestUi, label: &str) -> bool {
    ui::popup_item(&test.window, label, AccessibleRole::Button)
        .accessible_string_property(AccessibleStringProperty::Enabled)
        .is_some_and(|value| value == "true")
}

fn category(test: &mut ui::TestUi, index: usize) {
    ui::click_popup(&test.window, "Menu category", AccessibleRole::Combobox);
    for _ in 0..index {
        ui::key(&test.window, Key::DownArrow);
    }
    ui::key(&test.window, Key::Return);
    test.draw();
}

#[test]
fn parent_and_unsupported_archive_actions_are_disabled_without_dispatch() {
    let mut test = ui::TestUi::new();
    let mutations = Rc::new(RefCell::new(Vec::new()));
    let captured = mutations.clone();
    test.window
        .on_mutation(move |pane, action| captured.borrow_mut().push((pane, action)));
    test.window.invoke_show_app_menu(2);
    test.draw();
    for label in [
        "Rename…",
        "View",
        "Copy to…",
        "Move to Trash…",
        "Delete permanently…",
    ] {
        assert!(!enabled(&test, label), "no real target: {label}");
    }
    assert!(enabled(&test, "Create folder…"));
    assert!(
        enabled(&test, "Checksums…"),
        "Opening manifest verification needs no file selection"
    );
    ui::click_popup(&test.window, "Rename…", AccessibleRole::Button);
    assert!(mutations.borrow().is_empty());
    assert!(ui::has_popup(&test.window));
    // Selection can target real entries even while the caret remains on '..'.
    test.window.set_left_can_target_selection(true);
    test.draw();
    assert!(enabled(&test, "Move to Trash…"));
    assert!(!enabled(&test, "Rename…"));
    test.window.set_left_can_target_cursor(true);
    test.window.set_left_native_filesystem(false);
    test.draw();
    assert!(enabled(&test, "View"));
    assert!(enabled(&test, "Copy to…"));
    assert!(enabled(&test, "Make File List…"));
    for label in [
        "Move to…",
        "Rename…",
        "Create folder…",
        "Delete permanently…",
        "Connect network drive…",
    ] {
        assert!(!enabled(&test, label), "unsupported location: {label}");
    }
    ui::key(&test.window, Key::Escape);
    assert!(!ui::has_popup(&test.window));
    assert_eq!(test.window.get_focused_pane(), 0);
}

#[test]
fn category_makes_application_actions_reachable_and_close_never_runs_them() {
    let mut test = ui::TestUi::new();
    test.resize(320, 200);
    test.window.invoke_show_app_menu(2);
    test.draw();
    category(&mut test, 5);
    render::snapshot(&test, "main-menu-application-320");
    let options = ui::popup_item(&test.window, "Options", AccessibleRole::Button);
    let origin = options.map_to_native_window(options.geometry().origin);
    assert!(origin.y >= 0.0 && origin.y + options.geometry().height() <= 200.0);
    ui::click_popup(&test.window, "Close menu", AccessibleRole::Button);
    test.draw();
    assert!(!ui::has_popup(&test.window));
    test.window.invoke_show_app_menu(2);
    test.draw();
    // Opening again resets the category and scroll offset to expose Files first.
    let view = ui::popup_item(&test.window, "View", AccessibleRole::Button);
    let origin = view.map_to_native_window(view.geometry().origin);
    assert!(origin.y >= 0.0 && origin.y + view.geometry().height() <= 200.0);
    ui::key(&test.window, Key::Escape);
}

#[test]
fn menu_hints_use_the_users_remapped_shortcuts() {
    use midnight_core::{Event, ShortcutBinding, UserPreferences};
    let mut test = ui::TestUi::new();
    let presenter = Rc::new(RefCell::new(midnight_ui::Presenter::new(
        std::env::temp_dir(),
        std::env::temp_dir(),
    )));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            shortcuts: vec![ShortcutBinding {
                action: "files.rename".into(),
                keys: vec!["Ctrl+Alt+R".into()],
            }],
            ..Default::default()
        }))));
    midnight_ui::install(&test.window, presenter.clone(), Rc::new(|_| true));
    midnight_ui::refresh(&test.window, &presenter.borrow());
    test.window.invoke_show_app_menu(2);
    test.draw();
    let rename = ui::popup_item(&test.window, "Rename…", AccessibleRole::Button);
    assert!(
        ui::visible_text_values(rename)
            .iter()
            .any(|text| text.contains("Ctrl+Alt+R"))
    );
    ui::key(&test.window, Key::Escape);
}

#[test]
fn window_commands_route_navigation_and_panel_view_modes() {
    let test = ui::TestUi::new();
    let choices = Rc::new(RefCell::new(Vec::new()));
    let captured = choices.clone();
    test.window.on_navigation_choice(move |pane, identity| {
        captured.borrow_mut().push((pane, identity.to_string()));
    });
    test.window
        .invoke_function_action("panel.as-other".into(), 1);
    assert_eq!(*choices.borrow(), [(1, "other-panel".into())]);
    for (action, expected) in [
        ("view.brief", 1),
        ("view.detailed", 0),
        ("view.thumbnails", 2),
    ] {
        test.window.invoke_function_action(action.into(), 1);
        assert_eq!(test.window.get_right_view_mode(), expected);
        assert_eq!(test.window.get_left_view_mode(), 0);
    }
}
