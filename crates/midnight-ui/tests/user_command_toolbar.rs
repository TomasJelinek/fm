#[path = "support/modeless.rs"]
mod modeless;
use i_slint_core::items::AccessibleRole;
use midnight_ui::{MainWindow, UserCommandRow, UserCommandSettingsView};
use modeless::{Windows, click, draw};
use slint::{
    ComponentHandle, Model,
    platform::{Key, PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

fn row(index: usize) -> UserCommandRow {
    UserCommandRow {
        identity: format!("command-{index}").into(),
        revision: "1".into(),
        name: format!("Custom command {index}").into(),
        enabled: true,
        ..Default::default()
    }
}

#[test]
fn command_toolbar_rejects_rebound_click_and_opens_its_settings_from_context_menu() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    let rows = Rc::new(slint::VecModel::from(vec![
        row(1),
        UserCommandRow {
            enabled: false,
            ..row(2)
        },
    ]));
    main.set_user_commands(rows.clone().into());
    let calls = Rc::new(RefCell::new(Vec::new()));
    main.on_execute_user_command({
        let calls = calls.clone();
        move |id, rev| calls.borrow_mut().push((id, rev))
    });
    let selected = Rc::new(RefCell::new(Vec::new()));
    main.global::<UserCommandSettingsView>().on_select({
        let selected = selected.clone();
        move |id| selected.borrow_mut().push(id)
    });
    main.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 320, 200, "command-toolbar-small");
    let button =
        modeless::accessible_item(&native, AccessibleRole::Button, "Custom command 1").unwrap();
    let rect = button.geometry();
    let origin = button.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    );
    native.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    rows.set_row_data(
        0,
        UserCommandRow {
            revision: "2".into(),
            ..row(1)
        },
    );
    native
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    assert!(
        calls.borrow().is_empty(),
        "a rebound button cannot execute a changed command"
    );
    click(&native, "Custom command 1");
    assert_eq!(
        calls.borrow().as_slice(),
        &[("command-1".into(), "2".into())]
    );
    click(&native, "Custom command 2");
    assert_eq!(calls.borrow().len(), 1, "disabled command must not execute");
    native.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Right,
    });
    native
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Right,
        });
    draw(&native, 320, 200, "command-toolbar-context");
    assert!(
        !i_slint_core::window::WindowInner::from_pub(native.window())
            .active_popups()
            .is_empty(),
        "a real right click must open the toolbar context menu"
    );
    for key in [Key::DownArrow, Key::Return] {
        native
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        native
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    }
    draw(&native, 320, 200, "command-settings-small");
    assert_eq!(main.get_options_topic_index(), 9);
    assert_eq!(
        calls.borrow().len(),
        1,
        "Settings must not execute the command"
    );
    assert_eq!(
        selected.borrow().as_slice(),
        &[slint::SharedString::from("command-1")]
    );
    main.hide().unwrap();
}

#[test]
fn overflowing_toolbar_keeps_minimum_window_and_reveals_later_command_icons() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    main.set_user_commands(
        Rc::new(slint::VecModel::from((1..=40).map(row).collect::<Vec<_>>())).into(),
    );
    let visible = Rc::new(RefCell::new(std::collections::HashSet::new()));
    main.on_user_command_icon_visible({
        let visible = visible.clone();
        move |id, _, showing| {
            if showing {
                visible.borrow_mut().insert(id);
            } else {
                visible.borrow_mut().remove(&id);
            }
        }
    });
    main.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 320, 200, "command-toolbar-overflow");
    assert!(
        visible
            .borrow()
            .contains(&slint::SharedString::from("command-1"))
    );
    assert!(
        !visible
            .borrow()
            .contains(&slint::SharedString::from("command-40"))
    );
    for _ in 0..16 {
        click(&native, "Scroll commands right");
        draw(&native, 320, 200, "command-toolbar-end");
    }
    assert!(
        visible
            .borrow()
            .contains(&slint::SharedString::from("command-40"))
    );
    assert!(
        !visible
            .borrow()
            .contains(&slint::SharedString::from("command-1"))
    );
    main.hide().unwrap();
}
