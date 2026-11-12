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
        "Select files and folders",
        "Select files only",
        "Deselect files only",
        "Deselect files and folders",
        "Invert files only",
        "Invert files and folders",
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

    let selections = Rc::new(RefCell::new(Vec::new()));
    let captured = selections.clone();
    test.window.on_bulk_selection(move |pane, operation| {
        captured.borrow_mut().push((pane, operation));
    });
    for label in ["Invert files only", "Invert files and folders"] {
        test.window.invoke_show_app_menu(2);
        test.draw();
        category(&mut test, 2);
        ui::click_popup(&test.window, label, AccessibleRole::Button);
        test.draw();
        assert!(!ui::has_popup(&test.window));
    }
    assert_eq!(*selections.borrow(), [(1, 2), (1, 4)]);
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

fn shortcut_menu(bindings: &[(&str, &[&str])]) -> ui::TestUi {
    use midnight_core::{Event, ShortcutBinding, UserPreferences};
    let mut test = ui::TestUi::new();
    let presenter = Rc::new(RefCell::new(midnight_ui::Presenter::new(
        std::env::temp_dir(),
        std::env::temp_dir(),
    )));
    let preferences = UserPreferences {
        shortcuts: bindings
            .iter()
            .map(|(action, keys)| ShortcutBinding {
                action: (*action).into(),
                keys: keys.iter().map(|key| (*key).into()).collect(),
            })
            .collect(),
        ..Default::default()
    };
    preferences.validate().unwrap();
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
    midnight_ui::install(&test.window, presenter.clone(), Rc::new(|_| true));
    midnight_ui::refresh(&test.window, &presenter.borrow());
    test.window.set_left_can_target_cursor(true);
    test.window.invoke_focus_files();
    ui::key(&test.window, Key::F10);
    test.draw();
    assert!(
        ui::has_popup(&test.window),
        "F10 must open the existing menu"
    );
    test
}

fn menu_shortcut(test: &ui::TestUi, label: &str) -> Vec<String> {
    let item = ui::popup_item(&test.window, label, AccessibleRole::Button);
    ui::visible_text_values(item)
        .into_iter()
        .filter(|text| !text.is_empty() && text != label)
        .map(|text| text.to_string())
        .collect()
}

#[test]
fn selection_menu_shows_current_defaults_and_dispatches_exact_file_or_folder_scope() {
    let mut test = shortcut_menu(&[]);
    ui::key(&test.window, Key::Escape);
    test.window.set_active_pane(1);
    let selections = Rc::new(RefCell::new(Vec::new()));
    let captured = selections.clone();
    test.window.on_bulk_selection(move |pane, operation| {
        captured.borrow_mut().push((pane, operation));
    });
    for (label, shortcut, operation) in [
        ("Select files and folders", "Ctrl+Shift+A", 0),
        ("Select files only", "Ctrl+A", 3),
        ("Deselect files only", "Ctrl+D", 5),
        ("Deselect files and folders", "Ctrl+Shift+D", 1),
    ] {
        test.window.invoke_show_app_menu(2);
        test.draw();
        category(&mut test, 2);
        assert_eq!(menu_shortcut(&test, label), [shortcut], "{label}");
        render::snapshot(&test, "main-menu-selection-defaults");
        ui::click_popup(&test.window, label, AccessibleRole::Button);
        test.draw();
        assert!(!ui::has_popup(&test.window));
        assert_eq!(selections.borrow().last(), Some(&(1, operation)), "{label}");
    }
    assert_eq!(*selections.borrow(), [(1, 0), (1, 3), (1, 5), (1, 1)]);
}

#[test]
fn menu_hints_use_remapped_shortcuts_and_update_while_the_menu_stays_open() {
    let mut test = shortcut_menu(&[("files.rename", &["Ctrl+Alt+R"])]);
    assert_eq!(menu_shortcut(&test, "Rename…"), ["Ctrl+Alt+R"]);
    test.window
        .invoke_keyboard_apply("files.rename".into(), "Ctrl+Alt+T".into());
    test.draw();
    assert!(ui::has_popup(&test.window));
    assert_eq!(menu_shortcut(&test, "Rename…"), ["Ctrl+Alt+T"]);
    render::snapshot(&test, "main-menu-live-remapped");

    test.window
        .invoke_keyboard_apply("files.rename".into(), "".into());
    test.draw();
    assert!(ui::has_popup(&test.window));
    assert!(
        menu_shortcut(&test, "Rename…").is_empty(),
        "an unassigned command must not advertise its old key or the default F2"
    );
    render::snapshot(&test, "main-menu-unassigned");

    test.window.invoke_keyboard_restore("files.rename".into());
    test.draw();
    assert!(ui::has_popup(&test.window));
    assert_eq!(menu_shortcut(&test, "Rename…"), ["F2"]);
    ui::key(&test.window, Key::Escape);
    assert!(!ui::has_popup(&test.window));
}

#[test]
fn menu_suppresses_a_new_default_key_reassigned_to_another_action_and_restores_it() {
    let mut test = shortcut_menu(&[]);
    assert_eq!(
        menu_shortcut(&test, "Deselect files and folders"),
        ["Ctrl+Shift+D"]
    );
    assert_eq!(menu_shortcut(&test, "Rename…"), ["F2"]);
    // A newly introduced default yields to an explicit user assignment. Existing
    // defaults such as Edit/F4 still require their conflicting binding to be removed.
    test.window
        .invoke_keyboard_apply("files.rename".into(), "Ctrl+Shift+D".into());
    test.draw();
    assert!(
        test.window.get_keyboard_error().is_empty(),
        "{}",
        test.window.get_keyboard_error()
    );
    assert!(ui::has_popup(&test.window));
    assert_eq!(menu_shortcut(&test, "Rename…"), ["Ctrl+Shift+D"]);
    assert!(
        menu_shortcut(&test, "Deselect files and folders").is_empty(),
        "the suppressed default must not advertise a chord now assigned to Rename"
    );
    test.window.invoke_keyboard_restore_all();
    test.draw();
    assert!(ui::has_popup(&test.window));
    assert_eq!(menu_shortcut(&test, "Rename…"), ["F2"]);
    assert_eq!(
        menu_shortcut(&test, "Deselect files and folders"),
        ["Ctrl+Shift+D"]
    );
    ui::key(&test.window, Key::Escape);
}

#[test]
fn menu_shows_the_primary_alias_and_its_hover_hint_lists_every_alias() {
    use i_slint_core::{
        item_tree::ItemRc,
        window::{WindowInner, WindowKind},
    };
    use slint::{ComponentHandle, platform::WindowEvent};
    let mut test = shortcut_menu(&[("files.rename", &["Ctrl+Alt+R", "F2"])]);
    assert_eq!(
        menu_shortcut(&test, "Rename…"),
        ["F2"],
        "the menu column uses the preferred function key, not all aliases"
    );
    let rename = ui::popup_item(&test.window, "Rename…", AccessibleRole::Button);
    let rect = rename.geometry();
    let origin = rename.map_to_native_window(rect.origin);
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(
                origin.x + rect.width() / 2.0,
                origin.y + rect.height() / 2.0,
            ),
        });
    std::thread::sleep(std::time::Duration::from_millis(650));
    test.draw();
    let text = {
        let inner = WindowInner::from_pub(test.window.window());
        let popups = inner.active_popups();
        let tooltip = popups
            .iter()
            .find(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
            .expect("hovering the menu row must show its actual tooltip");
        ui::visible_text_values(ItemRc::new_root(tooltip.component.clone()))
    };
    assert!(
        text.iter().any(|text| text == "Rename… (Ctrl+Alt+R / F2)"),
        "the rendered hint must include both effective aliases: {text:?}"
    );
    render::snapshot(&test, "main-menu-alias-tooltip");
    drop(rename);
    ui::key(&test.window, Key::Escape);
    assert!(!ui::has_popup(&test.window));
}

#[test]
fn view_shortcuts_change_only_the_active_pane_and_leave_command_editing_alone() {
    let mut test = shortcut_menu(&[]);
    ui::key(&test.window, Key::Escape);
    for pane in 0..2 {
        test.window.invoke_activate(pane);
        test.window.invoke_focus_files();
        assert_eq!(test.window.get_focused_pane(), pane);
        for (key, mode) in [("2", 1), ("3", 0), ("5", 2)] {
            let before = [
                test.window.get_left_view_mode(),
                test.window.get_right_view_mode(),
            ];
            ui::chord(&test.window, &[Key::Alt], key);
            test.draw();
            let after = [
                test.window.get_left_view_mode(),
                test.window.get_right_view_mode(),
            ];
            assert_eq!(after[pane as usize], mode, "pane {pane}: Alt+{key}");
            assert_eq!(after[1 - pane as usize], before[1 - pane as usize]);
            assert_eq!(test.window.get_active_pane(), pane);
        }
    }
    for (key, mode, action) in [
        ("2", 1, "view.brief"),
        ("3", 0, "view.detailed"),
        ("5", 2, "view.thumbnails"),
    ] {
        // The test backend supplies a text-bearing Alt key event. Native editors
        // may accept that text (including Alt/Option layout input), so compare
        // against the same widget with this application binding disabled.
        test.window.invoke_keyboard_apply(action.into(), "".into());
        test.draw();
        assert!(test.window.get_keyboard_error().is_empty());
        test.window.invoke_set_command("kept view draft".into(), 15);
        ui::chord(&test.window, &[Key::Alt], key);
        test.draw();
        let native_text = test.window.get_command_text();
        assert_eq!(test.window.get_focused_pane(), -1);
        test.window.invoke_keyboard_restore(action.into());
        test.draw();
        assert!(test.window.get_keyboard_error().is_empty());

        // A different mode makes accidental panel dispatch observable.
        test.window.invoke_view_mode(1, (mode + 1) % 3);
        test.window.invoke_set_command("kept view draft".into(), 15);
        assert_eq!(test.window.get_focused_pane(), -1);
        let before = [
            test.window.get_left_view_mode(),
            test.window.get_right_view_mode(),
        ];
        ui::chord(&test.window, &[Key::Alt], key);
        test.draw();
        assert_eq!(
            [
                test.window.get_left_view_mode(),
                test.window.get_right_view_mode()
            ],
            before
        );
        assert_eq!(test.window.get_command_text(), native_text);
        assert_eq!(test.window.get_focused_pane(), -1);
    }
}

#[test]
fn view_menu_hints_and_key_dispatch_follow_live_remapping() {
    let mut test = shortcut_menu(&[]);
    category(&mut test, 3);
    for (label, key) in [
        ("Brief view", "Alt+2"),
        ("Detailed view", "Alt+3"),
        ("Thumbnail view", "Alt+5"),
    ] {
        assert_eq!(menu_shortcut(&test, label), [key]);
    }
    test.window
        .invoke_keyboard_apply("view.brief".into(), "Ctrl+Shift+R".into());
    test.draw();
    assert!(
        test.window.get_keyboard_error().is_empty(),
        "{}",
        test.window.get_keyboard_error()
    );
    assert!(ui::has_popup(&test.window));
    assert_eq!(menu_shortcut(&test, "Brief view"), ["Ctrl+Shift+R"]);
    assert_eq!(menu_shortcut(&test, "Detailed view"), ["Alt+3"]);
    assert_eq!(menu_shortcut(&test, "Thumbnail view"), ["Alt+5"]);
    render::snapshot(&test, "main-menu-view-shortcuts-remapped");
    ui::key(&test.window, Key::Escape);
    test.window.invoke_activate(0);
    test.window.invoke_focus_files();
    assert_eq!(test.window.get_left_view_mode(), 0);
    ui::chord(&test.window, &[Key::Alt], "2");
    test.draw();
    assert_eq!(
        test.window.get_left_view_mode(),
        0,
        "the replaced default must stop dispatching"
    );
    ui::chord(&test.window, &[Key::Control, Key::Shift], "r");
    test.draw();
    assert_eq!(test.window.get_left_view_mode(), 1);
    assert_eq!(test.window.get_right_view_mode(), 0);
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

#[test]
fn panel_view_and_sort_dropdowns_show_live_shortcuts_without_changing_labels() {
    let mut test = shortcut_menu(&[]);
    ui::key(&test.window, Key::Escape);
    for pane in ["Left", "Right"] {
        ui::click_root(
            &test.window,
            &format!("{pane} panel view mode"),
            AccessibleRole::Combobox,
        );
        test.draw();
        for (label, keys) in [
            ("Detailed", "Alt+3"),
            ("Brief", "Alt+2"),
            ("Thumbnails", "Alt+5"),
        ] {
            let text = ui::visible_text_values(ui::popup_item(
                &test.window,
                label,
                AccessibleRole::Button,
            ));
            assert!(
                text.iter().any(|value| value == keys),
                "{pane} {label}: {text:?}"
            );
        }
        ui::key(&test.window, Key::Escape);
        ui::click_root(
            &test.window,
            &format!("{pane} panel sort"),
            AccessibleRole::Combobox,
        );
        test.draw();
        for (label, keys) in [
            ("Name", "Ctrl+F3"),
            ("Size", "Ctrl+F6"),
            ("Modified", "Ctrl+F5"),
            ("Extension", "Ctrl+F4"),
            ("Reverse order", "Ctrl+F3"),
        ] {
            let text = ui::visible_text_values(ui::popup_item(
                &test.window,
                label,
                AccessibleRole::Button,
            ));
            assert!(
                text.iter().any(|value| value == keys),
                "{pane} {label}: {text:?}"
            );
        }
        ui::key(&test.window, Key::Escape);
    }
    ui::click_root(
        &test.window,
        "Left panel view mode",
        AccessibleRole::Combobox,
    );
    test.window
        .invoke_keyboard_apply("view.brief".into(), "Ctrl+Shift+R".into());
    test.draw();
    assert!(test.window.get_keyboard_error().is_empty());
    assert!(ui::has_popup(&test.window));
    let text = ui::visible_text_values(ui::popup_item(
        &test.window,
        "Brief",
        AccessibleRole::Button,
    ));
    assert!(text.iter().any(|value| value == "Ctrl+Shift+R"));
    assert!(!text.iter().any(|value| value == "Alt+2"));
    ui::click_popup(&test.window, "Brief", AccessibleRole::Button);
    test.draw();
    assert_eq!(test.window.get_left_view_mode(), 1);
    ui::click_root(
        &test.window,
        "Left panel view mode",
        AccessibleRole::Combobox,
    );
    test.window
        .invoke_keyboard_apply("view.brief".into(), "".into());
    test.draw();
    let text = ui::visible_text_values(ui::popup_item(
        &test.window,
        "Brief",
        AccessibleRole::Button,
    ));
    assert!(
        !text
            .iter()
            .any(|value| value.contains("Alt+") || value.contains("Ctrl+"))
    );
    render::snapshot(&test, "panel-view-dropdown-shortcuts");
}
