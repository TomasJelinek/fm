#[path = "support/ui.rs"]
mod ui;
use i_slint_core::items::AccessibleRole;
use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

fn control_m(window: &midnight_ui::MainWindow) {
    for event in [
        WindowEvent::KeyPressed {
            text: Key::Control.into(),
        },
        WindowEvent::KeyPressed { text: "m".into() },
        WindowEvent::KeyReleased { text: "m".into() },
        WindowEvent::KeyReleased {
            text: Key::Control.into(),
        },
    ] {
        window.window().dispatch_event(event);
    }
}

#[test]
fn file_list_key_menu_and_toolbar_dispatch_active_archive_panel_but_not_editor() {
    let mut test = ui::TestUi::new();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let captured = calls.clone();
    test.window
        .on_make_file_list(move |pane| captured.borrow_mut().push(pane));
    test.window.set_active_pane(1);
    test.window.set_right_can_target_selection(true);
    test.window.set_right_native_filesystem(false);
    test.window.invoke_focus_files();
    control_m(&test.window);
    assert_eq!(*calls.borrow(), [1]);
    test.window.invoke_focus_command();
    control_m(&test.window);
    assert_eq!(*calls.borrow(), [1]);
    test.window.invoke_focus_files();
    test.draw();
    ui::click_root(&test.window, "Make File List", AccessibleRole::Button);
    assert_eq!(*calls.borrow(), [1, 1]);
    test.window.invoke_show_app_menu(2);
    test.draw();
    ui::click_popup(&test.window, "Menu category", AccessibleRole::Combobox);
    ui::key(&test.window, Key::Home);
    for _ in 0..4 {
        ui::key(&test.window, Key::DownArrow);
    }
    ui::key(&test.window, Key::Return);
    test.draw();
    ui::click_popup(&test.window, "Make File List…", AccessibleRole::Button);
    assert_eq!(*calls.borrow(), [1, 1, 1]);
}
