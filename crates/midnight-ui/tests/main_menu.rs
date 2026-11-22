//! F10 exposes commands through the same callbacks as toolbar and keyboard actions, and
//! shows every chord in force for each command, never cut and never under the scroll bar.
#[path = "support/render.rs"]
mod render;
#[path = "support/shortcut_editor.rs"]
mod shortcut_editor;
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, ComplexText, Flickable, Item, SimpleText, TextWrap},
    layout::Orientation,
    lengths::LogicalRect,
    window::{WindowInner, WindowKind},
};
use shortcut_editor::Shortcuts;
use slint::{ComponentHandle, platform::Key};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

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
    // Every main-menu action is dispatched through function-action.
    let ran = Rc::new(RefCell::new(Vec::<String>::new()));
    let record = ran.clone();
    test.window
        .on_function_action(move |action, _| record.borrow_mut().push(action.to_string()));
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
    assert!(ran.borrow().is_empty(), "Close menu ran {:?}", ran.borrow());
    test.window.invoke_show_app_menu(2);
    test.draw();
    // Opening again resets the category and scroll offset to expose Files first.
    let view = ui::popup_item(&test.window, "View", AccessibleRole::Button);
    let origin = view.map_to_native_window(view.geometry().origin);
    assert!(origin.y >= 0.0 && origin.y + view.geometry().height() <= 200.0);
    ui::key(&test.window, Key::Escape);
}

fn shortcut_menu(bindings: &[(&str, &[&str])]) -> (ui::TestUi, Shortcuts) {
    shortcut_menu_on(ui::TestUi::new(), bindings)
}

fn shortcut_menu_on(mut test: ui::TestUi, bindings: &[(&str, &[&str])]) -> (ui::TestUi, Shortcuts) {
    use midnight_core::{Event, ShortcutBinding, UserPreferences};
    let mut presenter = midnight_ui::Presenter::new(std::env::temp_dir(), std::env::temp_dir());
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
    presenter.apply(Event::PreferencesLoaded(Ok(Box::new(
        midnight_core::AcceptedPreferences {
            preferences,
            comparators: midnight_core::FileComparatorSnapshot::default(),
        },
    ))));
    let shortcuts = Shortcuts::install(&test.window, presenter);
    test.window.set_left_can_target_cursor(true);
    open_menu(&mut test);
    (test, shortcuts)
}

fn open_menu(test: &mut ui::TestUi) {
    test.window.invoke_focus_files();
    ui::key(&test.window, Key::F10);
    test.draw();
    assert!(
        ui::has_popup(&test.window),
        "F10 must open the existing menu"
    );
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
    let (mut test, _) = shortcut_menu(&[]);
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
        ("Deselect files and folders", "Ctrl+D", 1),
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
    assert_eq!(*selections.borrow(), [(1, 0), (1, 3), (1, 1)]);
}

#[test]
fn menu_hints_use_remapped_shortcuts_and_update_while_the_menu_stays_open() {
    let (mut test, shortcuts) = shortcut_menu(&[("files.rename", &["Ctrl+Alt+R"])]);
    assert_eq!(menu_shortcut(&test, "Rename…"), ["Ctrl+Alt+R"]);
    // The keyboard editor's Remove and Restore act while the menu stays open.
    shortcuts.unassign(&test.window, "files.rename");
    test.draw();
    assert!(ui::has_popup(&test.window));
    assert!(
        menu_shortcut(&test, "Rename…").is_empty(),
        "an unassigned command must not advertise its old key or the default F2"
    );
    render::snapshot(&test, "main-menu-unassigned");

    shortcuts.restore(&test.window, "files.rename");
    test.draw();
    assert!(ui::has_popup(&test.window));
    assert_eq!(menu_shortcut(&test, "Rename…"), ["F2"]);
    ui::key(&test.window, Key::Escape);
    assert!(!ui::has_popup(&test.window));

    // Assigning a new chord needs Options; the menu then advertises that chord.
    shortcuts.assign(&test.window, "files.rename", &["Ctrl+Alt+T"]);
    open_menu(&mut test);
    assert_eq!(menu_shortcut(&test, "Rename…"), ["Ctrl+Alt+T"]);
    render::snapshot(&test, "main-menu-live-remapped");
    ui::key(&test.window, Key::Escape);
    assert!(!ui::has_popup(&test.window));
}

#[test]
fn menu_suppresses_a_new_default_key_reassigned_to_another_action_and_restores_it() {
    // A newly introduced default yields to an explicit user assignment saved before
    // that default existed. Existing defaults such as Edit/F4 still require their
    // conflicting binding to be removed.
    let (mut test, shortcuts) = shortcut_menu(&[("files.rename", &["Ctrl+D"])]);
    assert_eq!(menu_shortcut(&test, "Rename…"), ["Ctrl+D"]);
    assert!(
        menu_shortcut(&test, "Deselect files and folders").is_empty(),
        "the suppressed default must not advertise a chord now assigned to Rename"
    );
    // The keyboard editor's Reset all restores both defaults in the open menu.
    shortcuts.reset_all(&test.window);
    test.draw();
    assert!(ui::has_popup(&test.window));
    assert_eq!(menu_shortcut(&test, "Rename…"), ["F2"]);
    assert_eq!(
        menu_shortcut(&test, "Deselect files and folders"),
        ["Ctrl+D"]
    );
    ui::key(&test.window, Key::Escape);
}

#[test]
fn menu_and_its_hover_hint_list_every_alias() {
    use slint::platform::WindowEvent;
    let (test, clock) = ui::TestUi::with_virtual_clock();
    let (mut test, _) = shortcut_menu_on(test, &[("files.rename", &["Ctrl+Alt+R", "F2"])]);
    assert_eq!(
        menu_shortcut(&test, "Rename…"),
        ["Ctrl+Alt+R / F2"],
        "the menu lists every alias, in the order Options › Keyboard does"
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
    // Slint shows a tooltip after 500 ms of hover.
    ui::advance(&clock, std::time::Duration::from_millis(650));
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
    let (mut test, shortcuts) = shortcut_menu(&[]);
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
        shortcuts.unassign(&test.window, action);
        test.draw();
        test.window.invoke_set_command("kept view draft".into(), 15);
        ui::chord(&test.window, &[Key::Alt], key);
        test.draw();
        let native_text = test.window.get_command_text();
        assert_eq!(test.window.get_focused_pane(), -1);
        shortcuts.restore(&test.window, action);
        test.draw();

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
    let (mut test, shortcuts) = shortcut_menu(&[]);
    category(&mut test, 3);
    for (label, key) in [
        ("Brief view", "Alt+2"),
        ("Detailed view", "Alt+3"),
        ("Thumbnail view", "Alt+5"),
    ] {
        assert_eq!(menu_shortcut(&test, label), [key]);
    }
    // Removing the alias updates the open menu at once.
    shortcuts.unassign(&test.window, "view.brief");
    test.draw();
    assert!(ui::has_popup(&test.window));
    assert!(menu_shortcut(&test, "Brief view").is_empty());
    assert_eq!(menu_shortcut(&test, "Detailed view"), ["Alt+3"]);
    ui::key(&test.window, Key::Escape);
    shortcuts.assign(&test.window, "view.brief", &["Ctrl+Shift+R"]);
    open_menu(&mut test);
    category(&mut test, 3);
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
    let (mut test, shortcuts) = shortcut_menu(&[]);
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
    shortcuts.assign(&test.window, "view.brief", &["Ctrl+Shift+R"]);
    test.draw();
    ui::click_root(
        &test.window,
        "Left panel view mode",
        AccessibleRole::Combobox,
    );
    test.draw();
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
    // Removing the alias updates the open dropdown at once.
    shortcuts.unassign(&test.window, "view.brief");
    test.draw();
    assert!(ui::has_popup(&test.window));
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

#[test]
fn menu_lists_every_default_chord_of_an_action() {
    let (mut test, _) = shortcut_menu(&[]);
    // In the order and with the " / " that Options › Keyboard lists them. Each row is
    // read in its category, which shows it without scrolling.
    for (index, label, chords) in [
        (
            4,
            "Calculate / cancel folder sizes",
            "Ctrl+Shift+S / Ctrl+Shift+F10",
        ),
        (1, "Delete permanently…", "Shift+Delete / Shift+F8"),
        (1, "System properties", "Alt+Enter / Ctrl+Alt+Enter"),
        (1, "Rename…", "F2"),
    ] {
        category(&mut test, index);
        assert_eq!(menu_shortcut(&test, label), [chords], "{label}");
        ui::key(&test.window, Key::Escape);
        open_menu(&mut test);
    }
    category(&mut test, 4);
    render::snapshot(&test, "main-menu-every-chord");
    ui::key(&test.window, Key::Escape);
}

#[test]
fn a_rebinding_of_several_chords_shows_in_the_menu() {
    let (mut test, shortcuts) = shortcut_menu(&[]);
    assert_eq!(menu_shortcut(&test, "Rename…"), ["F2"]);
    ui::key(&test.window, Key::Escape);
    // Options › Keyboard gives Rename two chords; the menu lists both, in that order.
    shortcuts.assign(&test.window, "files.rename", &["Ctrl+Alt+T", "Ctrl+Alt+Y"]);
    open_menu(&mut test);
    assert_eq!(menu_shortcut(&test, "Rename…"), ["Ctrl+Alt+T / Ctrl+Alt+Y"]);
    // Removing them while the menu is open empties the row at once.
    shortcuts.unassign(&test.window, "files.rename");
    test.draw();
    assert!(ui::has_popup(&test.window));
    assert!(menu_shortcut(&test, "Rename…").is_empty());
    ui::key(&test.window, Key::Escape);
}

/// An item's bounds in window coordinates.
fn bounds(item: &ItemRc) -> LogicalRect {
    let rect = item.geometry();
    LogicalRect::new(item.map_to_native_window(rect.origin), rect.size)
}

/// Calls `f` for `item` and every item inside it.
fn visit(item: &ItemRc, f: &mut impl FnMut(&ItemRc)) {
    f(item);
    let mut child = item.first_child();
    while let Some(next) = child {
        visit(&next, f);
        child = next.next_sibling();
    }
}

/// The open menu's command rows: every button but the header's close button.
fn menu_rows(test: &ui::TestUi) -> Vec<(String, ItemRc)> {
    let inner = WindowInner::from_pub(test.window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("the open menu");
    let mut rows = Vec::new();
    visit(&ItemRc::new_root(popup.component.clone()), &mut |item| {
        if item.accessible_role() == AccessibleRole::Button
            && let Some(label) = item.accessible_string_property(AccessibleStringProperty::Label)
            && label != "Close menu"
        {
            rows.push((label.to_string(), item.clone()));
        }
    });
    rows
}

/// The bounds of the vertical scroll bar beside `row`: the sibling of the scroll
/// view's Flickable.
fn scroll_bar(row: &ItemRc) -> LogicalRect {
    let mut parent = row.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(item) = parent {
        if item.downcast::<Flickable>().is_some() {
            return bounds(&item.next_sibling().expect("the scroll view's bar"));
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    panic!("the menu rows are not in a scroll view")
}

/// Whether `item` shows in `row`: no clip between them hides it (`visible: false` is a
/// clip of no size). The rows that the menu has scrolled out of view count too.
fn shown_in(item: &ItemRc, row: &ItemRc) -> bool {
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor == *row {
            return true;
        }
        if ancestor.borrow().as_ref().clips_children() && ancestor.geometry().is_empty() {
            return false;
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    false
}

/// The texts a row shows, each with whether it wraps, whether the menu shows that row
/// at the moment or not.
fn shown_texts(row: &ItemRc) -> Vec<(String, bool, ItemRc)> {
    let mut texts = Vec::new();
    visit(row, &mut |item| {
        if !shown_in(item, row) {
            return;
        }
        if let Some(text) = item.downcast::<SimpleText>() {
            texts.push((text.as_pin_ref().text().to_string(), false, item.clone()));
        } else if let Some(text) = item.downcast::<ComplexText>() {
            let text = text.as_pin_ref();
            let wraps = text.wrap() != TextWrap::NoWrap;
            texts.push((text.text().to_string(), wraps, item.clone()));
        }
    });
    texts.retain(|(text, _, _)| !text.is_empty());
    texts
}

/// A text's preferred extent along `orientation`, given `cross` across it (-1: any).
fn preferred(item: &ItemRc, orientation: Orientation, cross: f32) -> f32 {
    let adapter = item.window_adapter().expect("a shown text");
    match item.downcast::<SimpleText>() {
        Some(text) => {
            let text = text.as_pin_ref();
            text.layout_info(orientation, cross, &adapter, item)
                .preferred
        }
        None => {
            let text = item.downcast::<ComplexText>().expect("a text");
            let text = text.as_pin_ref();
            text.layout_info(orientation, cross, &adapter, item)
                .preferred
        }
    }
}

#[test]
fn no_menu_row_lies_under_the_scroll_bar_and_no_chord_is_cut() {
    let (mut test, _) = shortcut_menu(&[]);
    let mut problems = Vec::new();
    // Window, density and desktop and touch font sizes: the smallest window with each
    // density's default fonts, and the largest font Options accepts (32 px).
    for (width, height, density, desktop, touch) in [
        (1340, 820, 0, 12, 15),
        (320, 200, 0, 12, 15),
        (320, 200, 1, 12, 15),
        (320, 200, 0, 32, 32),
        (1340, 820, 1, 32, 32),
    ] {
        ui::key(&test.window, Key::Escape);
        assert!(!ui::has_popup(&test.window));
        test.window.set_density_index(density);
        test.window.invoke_apply_fonts("".into(), desktop, touch);
        test.resize(width, height);
        open_menu(&mut test);
        test.draw();
        let case = format!("{width}x{height}, density {density}, fonts {desktop}/{touch}");
        render::snapshot(
            &test,
            &format!("main-menu-keys-{width}x{height}-density-{density}-font-{desktop}"),
        );
        let rows = menu_rows(&test);
        if rows.len() < 60 {
            problems.push(format!("{case}: only {} rows", rows.len()));
            continue;
        }
        let bar = scroll_bar(&rows[0].1);
        for (label, row) in &rows {
            let area = bounds(row);
            if area.max_x() > bar.min_x() + 0.5 {
                problems.push(format!(
                    "{case}: {label:?} {area:?} reaches under the bar {bar:?}"
                ));
            }
            for (text, wraps, item) in shown_texts(row) {
                let shown = bounds(&item);
                if shown.max_x() > bar.min_x() + 0.5 {
                    problems.push(format!(
                        "{case}: {text:?} {shown:?} lies under the bar {bar:?}"
                    ));
                }
                // The label may elide; a chord is never cut.
                if text == *label {
                    continue;
                }
                let (needs, has) = if wraps {
                    let needs = preferred(&item, Orientation::Vertical, shown.width());
                    (needs, shown.height())
                } else {
                    (
                        preferred(&item, Orientation::Horizontal, -1.0),
                        shown.width(),
                    )
                };
                if needs > has + 0.5 {
                    problems.push(format!(
                        "{case}: {text:?} needs {needs:.1} px, has {has:.1}"
                    ));
                }
                if shown.min_x() < area.min_x() - 0.5
                    || shown.max_x() > area.max_x() + 0.5
                    || shown.max_y() > area.max_y() + 0.5
                {
                    problems.push(format!(
                        "{case}: {text:?} {shown:?} leaves its row {area:?}"
                    ));
                }
            }
        }
    }
    ui::key(&test.window, Key::Escape);
    assert!(problems.is_empty(), "{problems:#?}");
}

/// Scrolls the open menu until `row` is at its top, as the wheel does.
fn reveal(test: &mut ui::TestUi, row: &ItemRc) {
    let mut parent = row.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(item) = parent {
        if item.downcast::<Flickable>().is_some() {
            let view = bounds(&item);
            let offset = bounds(row).min_y() - view.min_y();
            test.window
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(view.min_x() + 20.0, view.min_y() + 4.0),
                    delta_x: 0.0,
                    delta_y: -offset,
                });
            test.draw();
            return;
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    panic!("the menu rows are not in a scroll view")
}

#[test]
fn a_row_too_narrow_for_its_label_and_chords_lists_them_whole_under_the_label() {
    let (mut test, _) = shortcut_menu(&[]);
    const LABEL: &str = "Calculate / cancel folder sizes";
    // Fonts: one line of chords fits under the label; with 32 px fonts one chord a line.
    for (desktop, touch, lines) in [
        (12, 15, &["Ctrl+Shift+S / Ctrl+Shift+F10"][..]),
        (32, 32, &["Ctrl+Shift+S", "Ctrl+Shift+F10"][..]),
    ] {
        ui::key(&test.window, Key::Escape);
        test.window.invoke_apply_fonts("".into(), desktop, touch);
        test.resize(320, 200);
        open_menu(&mut test);
        category(&mut test, 4);
        let row = ui::popup_item(&test.window, LABEL, AccessibleRole::Button);
        let texts = shown_texts(&row);
        let label = texts
            .iter()
            .find(|(text, ..)| text == LABEL)
            .map(|(.., item)| bounds(item))
            .expect("the label");
        let shown: Vec<&str> = texts
            .iter()
            .filter(|(text, ..)| text != LABEL)
            .map(|(text, ..)| text.as_str())
            .collect();
        assert_eq!(shown, lines, "fonts {desktop}");
        for (text, _, item) in texts.iter().filter(|(text, ..)| text != LABEL) {
            let area = bounds(item);
            assert!(
                area.min_y() >= label.max_y() - 0.5,
                "fonts {desktop}: {text:?} {area:?} is under the label {label:?}"
            );
            assert!(
                preferred(item, Orientation::Horizontal, -1.0) <= area.width() + 0.5,
                "fonts {desktop}: {text:?} is whole in {area:?}"
            );
        }
        reveal(&mut test, &row);
        render::snapshot(
            &test,
            &format!("main-menu-chords-under-the-label-320-font-{desktop}"),
        );
    }
    ui::key(&test.window, Key::Escape);
}

/// The elevated terminal's label in the menu: the shortcut catalogue's on this platform.
const ELEVATED_TERMINAL: &str = if cfg!(windows) {
    "Open terminal as administrator"
} else {
    "Open root terminal (sudo)"
};

#[test]
fn the_elevated_terminal_entry_follows_the_command_line_and_opens_the_shell_elevated() {
    let mut test = ui::TestUi::new();
    let normal = Rc::new(Cell::new(0));
    let elevated = Rc::new(Cell::new(0));
    let count = normal.clone();
    test.window
        .on_open_terminal(move || count.set(count.get() + 1));
    let count = elevated.clone();
    test.window
        .on_open_terminal_elevated(move || count.set(count.get() + 1));
    test.window.set_terminal_available(true);
    // Like the terminal button, the entry waits for the terminal to be ready.
    test.window.set_terminal_ready(false);
    open_menu(&mut test);
    let labels: Vec<String> = menu_rows(&test)
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    let line = labels
        .iter()
        .position(|label| label == "Command line")
        .expect("the command line entry");
    assert_eq!(
        labels.get(line + 1).map(String::as_str),
        Some(ELEVATED_TERMINAL),
        "the entry follows the command line: {labels:?}"
    );
    assert!(!enabled(&test, ELEVATED_TERMINAL));
    category(&mut test, 4);
    ui::click_popup(&test.window, ELEVATED_TERMINAL, AccessibleRole::Button);
    test.draw();
    assert_eq!((normal.get(), elevated.get()), (0, 0), "not while unready");
    ui::key(&test.window, Key::Escape);
    test.window.set_terminal_ready(true);
    open_menu(&mut test);
    category(&mut test, 4);
    assert!(enabled(&test, ELEVATED_TERMINAL));
    render::snapshot(&test, "main-menu-elevated-terminal");
    ui::click_popup(&test.window, ELEVATED_TERMINAL, AccessibleRole::Button);
    test.draw();
    assert!(!ui::has_popup(&test.window), "choosing it closes the menu");
    assert_eq!(
        (normal.get(), elevated.get()),
        (0, 1),
        "one elevated request, no plain one"
    );
}

#[test]
fn a_chord_bound_to_the_elevated_terminal_shows_in_the_menu() {
    let (mut test, _) = shortcut_menu(&[("command.open-terminal-elevated", &["Ctrl+Alt+T"])]);
    category(&mut test, 4);
    assert_eq!(menu_shortcut(&test, ELEVATED_TERMINAL), ["Ctrl+Alt+T"]);
    ui::key(&test.window, Key::Escape);
}
