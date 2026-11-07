//! Commander keys operate on the displayed action and keep editor state intact.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

#[test]
fn control_layer_exposes_configured_drive_information_action() {
    let mut test = ui::TestUi::new();
    test.window.invoke_focus_files();
    test.window
        .window()
        .dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
    test.draw();
    ui::root_item(&test.window, "Ctrl+F1 Drive info", AccessibleRole::Button);
    test.window
        .window()
        .dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
    test.draw();
    ui::root_item(&test.window, "F1 Help", AccessibleRole::Button);
}

#[test]
fn control_f11_expands_active_pane_and_restores_split_ratio() {
    let mut test = ui::TestUi::new();
    test.window.set_panel_ratio(0.61);
    test.window.invoke_focus_files();
    test.draw();
    let width = test.window.get_left_list_width();
    ui::chord(&test.window, &[Key::Control], Key::F11);
    test.draw();
    assert!(test.window.get_left_list_width() > width + 200.0);
    ui::chord(&test.window, &[Key::Control], Key::F11);
    test.draw();
    assert!((test.window.get_left_list_width() - width).abs() < 0.1);
    assert!((test.window.get_panel_ratio() - 0.61).abs() < 0.001);
}

#[test]
fn alt_history_wraps_without_changing_terminal_or_executing() {
    let mut test = ui::TestUi::new();
    let p = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let captured = sent.clone();
    install(
        &test.window,
        p.clone(),
        Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        }),
    );
    p.borrow_mut().apply(Event::DesktopContext {
        shell: ShellFlavor::PowerShell,
        locations: vec![],
    });
    p.borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            windows_shell: WindowsShell::Cmd,
            command_history: vec![
                CommandHistoryEntry {
                    command: "new 文档".into(),
                    shell: ShellFlavor::PowerShell,
                },
                CommandHistoryEntry {
                    command: "old 👩🏽‍💻".into(),
                    shell: ShellFlavor::Cmd,
                },
            ],
            ..Default::default()
        }))));
    refresh(&test.window, &p.borrow());
    test.window.invoke_set_command("draft".into(), 5);
    sent.borrow_mut().clear();
    for (key, expected) in [
        (Key::UpArrow, "new 文档"),
        (Key::UpArrow, "old 👩🏽‍💻"),
        (Key::UpArrow, "new 文档"),
        (Key::DownArrow, "old 👩🏽‍💻"),
    ] {
        ui::chord(&test.window, &[Key::Alt], key);
        test.draw();
        assert_eq!(test.window.get_command_text(), expected);
        assert_eq!(test.window.get_terminal_index(), 1);
        assert!(test.window.get_command_focused());
        assert!(sent.borrow().is_empty(), "recall never launches a terminal");
    }
    ui::key(&test.window, "x");
    ui::chord(&test.window, &[Key::Alt], Key::UpArrow);
    assert_eq!(
        test.window.get_command_text(),
        "new 文档",
        "editing resets traversal"
    );
    ui::key(&test.window, Key::Return);
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::LaunchTerminal { .. }))
    );
}
