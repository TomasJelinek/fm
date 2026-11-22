//! The terminal button's second action: a Ctrl+click, or Ctrl+Enter or Ctrl+Space with
//! keyboard focus, opens the selected shell elevated, and its tooltip says so.
use crate::support::ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, FocusReason, FocusScope},
    window::WindowInner,
};
use slint::{
    ComponentHandle, ModelRc, SharedString, VecModel,
    platform::{Key, WindowEvent},
};
use std::{cell::Cell, rc::Rc};

const WINDOWS: [&str; 2] = ["PowerShell", "cmd"];
const LINUX: [&str; 2] = ["Terminal", "Terminal (sudo)"];

fn configure(test: &mut ui::TestUi, windows: bool, index: i32, path: &str) {
    let choices = if windows { WINDOWS } else { LINUX };
    test.window.set_left_path(path.into());
    test.window
        .set_terminal_choices(ModelRc::new(VecModel::from(
            choices.map(SharedString::from).to_vec(),
        )));
    test.window.set_windows_shells(windows);
    test.window.set_terminal_index(index);
    test.window.set_terminal_available(true);
    test.window.set_terminal_ready(true);
    test.draw();
}

/// The button's accessible label is its tooltip.
fn label(hint: &str, path: &str) -> String {
    format!("{hint}\nWorking folder: {path}")
}

fn focus_scope(item: ItemRc) -> Option<ItemRc> {
    if item.is_visible() && item.downcast::<FocusScope>().is_some() {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(scope) = focus_scope(item) {
            return Some(scope);
        }
    }
    None
}

/// Gives the terminal button keyboard focus, as Tab would.
fn focus_button(test: &ui::TestUi, label: &str) {
    let button = ui::root_item(&test.window, label, AccessibleRole::Button);
    let scope = focus_scope(button).expect("the terminal button takes keyboard focus");
    let inner = WindowInner::from_pub(test.window.window());
    inner.set_focus_item(&scope, true, FocusReason::Programmatic);
    assert_eq!(inner.focus_item.borrow().upgrade().as_ref(), Some(&scope));
}

fn control(test: &ui::TestUi, pressed: bool) {
    let text = Key::Control.into();
    test.window.window().dispatch_event(if pressed {
        WindowEvent::KeyPressed { text }
    } else {
        WindowEvent::KeyReleased { text }
    });
}

#[test]
fn the_terminal_button_tooltip_names_the_shell_and_what_ctrl_click_opens() {
    let mut test = ui::TestUi::new();
    for (windows, index, path, hint) in [
        (
            true,
            0,
            "C:\\文档",
            "Open PowerShell in this folder. Ctrl+click: open it as administrator.",
        ),
        (
            true,
            1,
            "C:\\文档",
            "Open cmd in this folder. Ctrl+click: open it as administrator.",
        ),
        (
            false,
            0,
            "/srv/文档",
            "Open a terminal in this folder. Ctrl+click: open a root shell with sudo.",
        ),
        (
            false,
            1,
            "/srv/文档",
            "Open a root shell with sudo in this folder.",
        ),
    ] {
        configure(&mut test, windows, index, path);
        let label = label(hint, path);
        assert!(
            ui::try_root_item(&test.window, &label, AccessibleRole::Button).is_some(),
            "the terminal button's tooltip: {label:?}"
        );
    }
}

#[test]
fn ctrl_click_ctrl_enter_and_ctrl_space_open_elevated_and_plain_input_opens_normally() {
    let mut test = ui::TestUi::new();
    configure(&mut test, true, 1, "C:\\文档");
    let button = label(
        "Open cmd in this folder. Ctrl+click: open it as administrator.",
        "C:\\文档",
    );
    let normal = Rc::new(Cell::new(0));
    let elevated = Rc::new(Cell::new(0));
    let count = normal.clone();
    test.window
        .on_open_terminal(move || count.set(count.get() + 1));
    let count = elevated.clone();
    test.window
        .on_open_terminal_elevated(move || count.set(count.get() + 1));
    let opened = || (normal.get(), elevated.get());

    ui::click_root(&test.window, &button, AccessibleRole::Button);
    assert_eq!(opened(), (1, 0), "a plain click opens the shell as is");
    control(&test, true);
    ui::click_root(&test.window, &button, AccessibleRole::Button);
    control(&test, false);
    assert_eq!(opened(), (1, 1), "Ctrl+click opens it elevated");
    assert!(
        test.window.get_command_focused(),
        "like a plain click, it returns to the command editor"
    );
    ui::click_root(&test.window, &button, AccessibleRole::Button);
    assert_eq!(
        opened(),
        (2, 1),
        "a released Ctrl leaves the next click plain"
    );

    for (modifiers, key, expected) in [
        (&[][..], SharedString::from(Key::Return), (3, 1)),
        (&[][..], SharedString::from(" "), (4, 1)),
        (&[Key::Control][..], SharedString::from(Key::Return), (4, 2)),
        (&[Key::Control][..], SharedString::from(" "), (4, 3)),
        // Other modifiers leave the button alone.
        (&[Key::Shift][..], SharedString::from(Key::Return), (4, 3)),
        (
            &[Key::Control, Key::Alt][..],
            SharedString::from(Key::Return),
            (4, 3),
        ),
    ] {
        focus_button(&test, &button);
        ui::chord(&test.window, modifiers, key.clone());
        assert_eq!(
            opened(),
            expected,
            "{modifiers:?} {key:?} on the focused button"
        );
    }
}
