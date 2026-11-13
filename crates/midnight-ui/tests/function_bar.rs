//! Function captions describe the exact effective chord, including modifier layers.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_core::{Event, ShortcutBinding, UserPreferences};
use midnight_ui::{MainWindow, Presenter, ShortcutHints, install, refresh};
use slint::{
    ComponentHandle,
    platform::{Key, PointerEventButton},
};
use std::{cell::RefCell, rc::Rc};

const DEFAULT_ACTIONS: [[&str; 12]; 8] = [
    [
        "app.help",
        "files.rename",
        "files.view",
        "files.edit",
        "files.copy-to",
        "files.move-to",
        "files.create-directory",
        "files.trash",
        "app.user-menu",
        "app.main-menu",
        "network.connect",
        "network.disconnect",
    ],
    [
        "panel.drive-info",
        "files.attributes",
        "sort.name",
        "sort.extension",
        "sort.modified",
        "sort.size",
        "files.case",
        "files.checksums",
        "panel.refresh",
        "panel.compare",
        "panel.zoom",
        "panel.filter",
    ],
    [
        "drive.left",
        "drive.right",
        "files.view-basic",
        "app.exit",
        "",
        "",
        "files.find",
        "",
        "",
        "",
        "",
        "",
    ],
    [""; 12],
    [
        "",
        "",
        "",
        "files.create-file",
        "",
        "",
        "",
        "files.delete",
        "",
        "",
        "",
        "",
    ],
    [
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "panel.folder-sizes",
        "",
        "",
    ],
    [""; 12],
    [""; 12],
];

fn ready() -> ui::TestUi {
    let test = ui::TestUi::new();
    test.window.set_left_can_target_cursor(true);
    test.window.set_right_can_target_cursor(true);
    test.window.set_left_can_target_selection(true);
    test.window.set_right_can_target_selection(true);
    test.window.set_network_available(true);
    test.window.invoke_focus_files();
    test
}

fn modifiers(test: &ui::TestUi, mask: i32, pressed: bool) {
    let mut keys: Vec<_> = [
        (1, Key::Control),
        (2, Key::Alt),
        (4, Key::Shift),
        (8, Key::Meta),
    ]
    .into_iter()
    .filter(|(bit, _)| mask & bit != 0)
    .map(|(_, key)| key)
    .collect();
    if !pressed {
        keys.reverse();
    }
    for key in keys {
        ui::platform_keys::key_edge(&test.window, key.into(), pressed);
    }
}

fn chord(mask: i32, index: usize) -> String {
    let mut text = String::new();
    for (bit, modifier) in [(1, "Ctrl+"), (2, "Alt+"), (4, "Shift+"), (8, "Meta+")] {
        if mask & bit != 0 {
            text.push_str(modifier);
        }
    }
    format!("{text}F{}", index + 1)
}

fn visible_button(window: &MainWindow, label: &str) -> Option<ItemRc> {
    fn find(item: ItemRc, label: &str) -> Option<ItemRc> {
        if !item.is_visible() {
            return None;
        }
        if item.accessible_role() == AccessibleRole::Button
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|value| value == label)
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(found) = find(item, label) {
                return Some(found);
            }
        }
        None
    }
    find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        label,
    )
}

fn button_bounds(window: &MainWindow, label: &str) -> (f32, f32) {
    let item = visible_button(window, label).unwrap_or_else(|| panic!("missing button: {label}"));
    let rect = item.geometry();
    (item.map_to_native_window(rect.origin).x, rect.width())
}

#[test]
fn unavailable_network_dialogs_are_hidden_without_hiding_supported_modifier_actions() {
    let mut test = ready();
    test.draw();
    let full_width = button_bounds(&test.window, "F1 Help").1;
    assert!(visible_button(&test.window, "F11 Connect").is_some());
    assert!(visible_button(&test.window, "F12 Disconnect").is_some());
    test.window.set_network_available(false);
    test.draw();
    assert!(visible_button(&test.window, "F11 Connect — unavailable").is_none());
    assert!(visible_button(&test.window, "F12 Disconnect — unavailable").is_none());
    assert!(button_bounds(&test.window, "F1 Help").1 > full_width + 10.0);
    let menu = button_bounds(&test.window, "F10 Menu");
    let jobs = button_bounds(&test.window, "Jobs");
    assert!((jobs.0 - menu.0 - menu.1 - 4.0).abs() < 1.0);
    render::snapshot(&test, "function-bar-linux-no-network-dialogs");

    modifiers(&test, 1, true);
    test.draw();
    assert!(visible_button(&test.window, "Ctrl+F11 Zoom").is_some());
    assert!(visible_button(&test.window, "Ctrl+F12 Filter").is_some());
    modifiers(&test, 1, false);
    test.resize(320, 200);
    assert!(visible_button(&test.window, "F11 Connect — unavailable").is_none());
    assert!(visible_button(&test.window, "F12 Disconnect — unavailable").is_none());
    let jobs = button_bounds(&test.window, "Jobs");
    assert!(jobs.0 >= 0.0 && jobs.0 + jobs.1 <= 320.5);
    render::snapshot(&test, "function-bar-linux-no-network-dialogs-compact");
    test.resize(1340, 820);
    test.window.set_network_available(true);
    test.draw();
    assert!(visible_button(&test.window, "F11 Connect").is_some());
    assert!(visible_button(&test.window, "F12 Disconnect").is_some());
}

#[test]
fn unavailable_network_actions_follow_remapped_slots_and_close_the_gap() {
    let mut test = ready();
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            shortcuts: vec![
                ShortcutBinding {
                    action: "files.edit".into(),
                    keys: vec![],
                },
                ShortcutBinding {
                    action: "network.connect".into(),
                    keys: vec!["F4".into()],
                },
            ],
            ..Default::default()
        }))));
    install(&test.window, presenter.clone(), Rc::new(|_| true));
    refresh(&test.window, &presenter.borrow());
    test.window.set_network_available(false);
    test.draw();
    assert!(visible_button(&test.window, "F4 Connect — unavailable").is_none());
    let previous = button_bounds(&test.window, "F3 View — unavailable");
    let next = button_bounds(&test.window, "F5 Copy — unavailable");
    assert!((next.0 - previous.0 - previous.1 - 4.0).abs() < 1.0);
}

#[test]
fn hiding_network_actions_preserves_pressed_geometry_action_and_pane_until_release() {
    let mut test = ready();
    let copied = Rc::new(RefCell::new(Vec::new()));
    let output = copied.clone();
    test.window
        .on_file_clipboard(move |pane, operation| output.borrow_mut().push((pane, operation)));
    for (control, network_available) in [(true, false), (false, true)] {
        test.window.set_network_available(false);
        test.window.set_active_pane(0);
        test.window.invoke_focus_files();
        test.draw();
        let before = button_bounds(&test.window, "F5 Copy");
        let point = ui::root_position(&test.window, "F5 Copy", AccessibleRole::Button);
        test.window.invoke_capture_function_press(point.x, point.y);
        ui::pointer_button(&test.window, point, PointerEventButton::Left, true);
        test.window.set_active_pane(1);
        if control {
            modifiers(&test, 1, true);
        }
        test.window.set_network_available(network_available);
        test.draw();
        let label = if control {
            "Ctrl+F5 Modified"
        } else {
            "F5 Copy"
        };
        assert_eq!(button_bounds(&test.window, label), before);
        ui::pointer_button(&test.window, point, PointerEventButton::Left, false);
        test.window.invoke_release_function_press();
        test.draw();
        assert_ne!(button_bounds(&test.window, label), before);
        if control {
            modifiers(&test, 1, false);
        }
    }
    assert_eq!(&*copied.borrow(), &[(0, 2), (0, 2)]);
}

#[test]
fn jobs_stays_under_the_pointer_while_function_slots_change() {
    let mut test = ready();
    for (control, network_available) in [(true, false), (false, true)] {
        test.window.set_network_available(false);
        test.draw();
        let before = button_bounds(&test.window, "Jobs");
        let center = ui::root_position(&test.window, "Jobs", AccessibleRole::Button);
        let point = slint::LogicalPosition::new(before.0 + 5.0, center.y);
        test.window.invoke_capture_function_press(point.x, point.y);
        ui::pointer_button(&test.window, point, PointerEventButton::Left, true);
        if control {
            modifiers(&test, 1, true);
        }
        test.window.set_network_available(network_available);
        test.draw();
        assert_eq!(button_bounds(&test.window, "Jobs"), before);
        ui::pointer_button(&test.window, point, PointerEventButton::Left, false);
        test.window.invoke_release_function_press();
        test.draw();
        assert!(test.window.get_operations_visible());
        test.window.invoke_close_operations();
        if control {
            modifiers(&test, 1, false);
        }
    }
}

#[test]
fn every_function_modifier_layer_matches_the_effective_actions_and_visible_buttons() {
    let mut test = ready();
    for mask in 0..16 {
        let expected_actions = DEFAULT_ACTIONS
            .get(mask as usize)
            .copied()
            .unwrap_or([""; 12]);
        for searching in [false, true] {
            for (index, expected) in expected_actions.into_iter().enumerate() {
                let slot = test
                    .window
                    .global::<ShortcutHints>()
                    .invoke_lookup_function(index as i32, mask, searching);
                assert_eq!(
                    slot.action,
                    expected,
                    "{} searching={searching}",
                    chord(mask, index)
                );
                if !expected.is_empty() {
                    assert_eq!(slot.key, chord(mask, index));
                    assert!(!slot.label.is_empty());
                    assert!(slot.hint.contains(slot.key.as_str()));
                }
            }
        }
        if mask >= 8 {
            continue;
        }
        modifiers(&test, mask, true);
        test.draw();
        assert_eq!(
            test.window.get_function_modifiers(),
            mask,
            "requested layer {mask}; AltGr={}, Ctrl-only={}",
            test.window.get_native_alt_gr(),
            test.window.get_control_layer()
        );
        for index in 0..12 {
            let slot = test
                .window
                .global::<ShortcutHints>()
                .invoke_lookup_function(index as i32, mask, false);
            let label = if slot.action.is_empty() {
                format!("{} — unavailable", chord(mask, index))
            } else {
                format!("{} {}", chord(mask, index), slot.label)
            };
            let item = ui::root_item(&test.window, &label, AccessibleRole::Button);
            assert!(
                ui::visible_text_values(item)
                    .iter()
                    .any(|text| !text.is_empty()),
                "{label}"
            );
        }
        modifiers(&test, mask, false);
    }
    for (mask, index, label) in [
        (1, 3, "Extension"),
        (4, 3, "New file"),
        (4, 7, "Erase"),
        (2, 2, "Text view"),
        (2, 6, "Find"),
        (2, 3, "Exit"),
        (5, 9, "Dir sizes"),
        (2, 0, "Left drive"),
        (2, 1, "Right drive"),
    ] {
        assert_eq!(
            test.window
                .global::<ShortcutHints>()
                .invoke_lookup_function(index, mask, false)
                .label,
            label
        );
    }
    let erase = test
        .window
        .global::<ShortcutHints>()
        .invoke_lookup_function(7, 4, false);
    assert!(erase.hint.to_lowercase().contains("permanent"));
}

#[test]
fn control_f4_keeps_its_extension_caption_visible_at_desktop_widths() {
    let mut test = ready();
    modifiers(&test, 1, true);
    for width in [1340, 1600] {
        test.resize(width, 820);
        let item = ui::root_item(&test.window, "Ctrl+F4 Extension", AccessibleRole::Button);
        let text = ui::visible_text_values(item.clone());
        assert!(
            text.iter().any(|text| text.contains("Extension")),
            "visible Ctrl+F4 caption at {width}px: {text:?}"
        );
        let origin = item.map_to_native_window(item.geometry().origin);
        assert!(origin.x >= 0.0 && origin.x + item.geometry().width() <= width as f32);
        render::snapshot(&test, &format!("function-bar-control-extension-{width}"));
    }
    modifiers(&test, 1, false);
}

#[test]
fn modified_function_buttons_and_keyboard_dispatch_the_same_action_and_pane() {
    let mut test = ready();
    let actions = Rc::new(RefCell::new(Vec::<(String, i32, i32)>::new()));
    let out = actions.clone();
    test.window
        .on_sort(move |pane, column| out.borrow_mut().push(("sort".into(), pane, column)));
    let out = actions.clone();
    test.window.on_mutation(move |pane, operation| {
        out.borrow_mut().push(("mutation".into(), pane, operation))
    });
    let out = actions.clone();
    test.window.on_view_file(move |pane, basic| {
        out.borrow_mut()
            .push(("view".into(), pane, i32::from(basic)))
    });
    let out = actions.clone();
    test.window
        .on_find_files(move |pane| out.borrow_mut().push(("find".into(), pane, 0)));
    let out = actions.clone();
    test.window
        .on_folder_sizes(move |pane| out.borrow_mut().push(("sizes".into(), pane, 0)));
    for pane in [0, 1] {
        for (mask, key, label, action, value) in [
            (1, Key::F4, "Ctrl+F4 Extension", "sort", 3),
            (4, Key::F4, "Shift+F4 New file", "mutation", 4),
            (4, Key::F8, "Shift+F8 Erase", "mutation", 2),
            (2, Key::F3, "Alt+F3 Text view", "view", 1),
            (2, Key::F7, "Alt+F7 Find", "find", 0),
            (5, Key::F10, "Ctrl+Shift+F10 Dir sizes", "sizes", 0),
        ] {
            test.window.set_active_pane(pane);
            test.window.invoke_focus_files();
            modifiers(&test, mask, true);
            test.draw();
            let before = actions.borrow().len();
            ui::key(&test.window, key);
            assert_eq!(
                &actions.borrow()[before..],
                &[(action.into(), pane, value)],
                "keyboard {label}"
            );
            let before = actions.borrow().len();
            ui::click_root(&test.window, label, AccessibleRole::Button);
            assert_eq!(
                &actions.borrow()[before..],
                &[(action.into(), pane, value)],
                "mouse {label}"
            );
            modifiers(&test, mask, false);
        }
    }
}

#[test]
fn live_remapping_moves_the_physical_slot_and_respects_filename_search_scope() {
    let mut test = ready();
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(
            UserPreferences::default(),
        ))));
    install(&test.window, presenter.clone(), Rc::new(|_| true));
    refresh(&test.window, &presenter.borrow());
    test.window.set_left_can_target_cursor(true);
    test.window.set_left_can_target_selection(true);
    test.window.invoke_focus_files();
    test.window
        .invoke_keyboard_apply("files.rename".into(), "Shift+F5".into());
    test.window.set_left_can_target_cursor(true);
    assert!(test.window.get_keyboard_error().is_empty());
    test.draw();
    assert!(
        test.window
            .global::<ShortcutHints>()
            .invoke_lookup_function(1, 0, false)
            .action
            .is_empty()
    );
    ui::root_item(&test.window, "F2 — unavailable", AccessibleRole::Button);
    modifiers(&test, 4, true);
    test.draw();
    ui::root_item(&test.window, "Shift+F5 Rename", AccessibleRole::Button);
    let captured = Rc::new(RefCell::new(Vec::new()));
    let out = captured.clone();
    test.window
        .on_mutation(move |pane, operation| out.borrow_mut().push((pane, operation)));
    ui::key(&test.window, Key::F5);
    ui::click_root(&test.window, "Shift+F5 Rename", AccessibleRole::Button);
    assert_eq!(&*captured.borrow(), &[(0, 0), (0, 0)]);
    modifiers(&test, 4, false);
    test.window
        .invoke_keyboard_apply("files.attributes".into(), "".into());
    assert!(test.window.get_keyboard_error().is_empty());
    test.window
        .invoke_keyboard_apply("files.copy".into(), "Ctrl+F2".into());
    test.window.set_left_can_target_selection(true);
    assert!(test.window.get_keyboard_error().is_empty());
    modifiers(&test, 1, true);
    test.draw();
    ui::root_item(&test.window, "Ctrl+F2 Copy", AccessibleRole::Button);
    assert_eq!(
        test.window
            .global::<ShortcutHints>()
            .invoke_lookup_function(1, 1, false)
            .action,
        "files.copy"
    );
    assert!(
        test.window
            .global::<ShortcutHints>()
            .invoke_lookup_function(1, 1, true)
            .action
            .is_empty()
    );
    test.window.set_left_search_query("x".into());
    test.draw();
    ui::root_item(
        &test.window,
        "Ctrl+F2 — unavailable",
        AccessibleRole::Button,
    );
    modifiers(&test, 1, false);
}
