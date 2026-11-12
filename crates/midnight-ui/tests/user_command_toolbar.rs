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
    main.set_preferences_ready(true);
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
    draw(&native, 640, 420, "command-toolbar-normal");
    assert!(!main.get_compact_window());
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
    draw(&native, 640, 420, "command-toolbar-context");
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
    draw(&native, 640, 420, "command-settings-normal");
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
    let visibility_events = Rc::new(RefCell::new(Vec::new()));
    main.on_user_command_icon_visible({
        let visible = visible.clone();
        let visibility_events = visibility_events.clone();
        move |id, _, showing| {
            visibility_events.borrow_mut().push((id.clone(), showing));
            if showing {
                visible.borrow_mut().insert(id);
            } else {
                visible.borrow_mut().remove(&id);
            }
        }
    });
    main.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 320, 200, "command-toolbar-minimum");
    assert!(main.get_compact_window());
    assert_eq!(main.get_effective_minimum_height(), 200.0);
    let root = i_slint_core::window::WindowInner::from_pub(main.window()).component();
    assert_eq!(
        i_slint_core::item_tree::ItemTreeRc::borrow_pin(&root)
            .as_ref()
            .layout_info(i_slint_core::layout::Orientation::Horizontal)
            .min,
        320.0
    );
    // Compact windows expose the shared popup instead of the separate row.
    // shared_toolbar's tiny-window test exercises reachability, execution and pane capture.
    assert!(
        modeless::accessible_item(&native, AccessibleRole::Button, "Custom commands").is_some()
    );
    assert!(
        modeless::accessible_item(&native, AccessibleRole::Button, "Custom command 1").is_none()
    );
    draw(&native, 640, 420, "command-toolbar-overflow");
    // Visibility changes measured during layout reach the asynchronous icon
    // callback on the next event-loop iteration.
    slint::platform::update_timers_and_animations();
    assert!(!main.get_compact_window());
    assert!(
        visible
            .borrow()
            .contains(&slint::SharedString::from("command-1")),
        "initial icon visibility events: {:?}",
        visibility_events.borrow()
    );
    assert!(
        !visible
            .borrow()
            .contains(&slint::SharedString::from("command-40"))
    );
    for _ in 0..16 {
        click(&native, "Scroll commands right");
        draw(&native, 640, 420, "command-toolbar-end");
        slint::platform::update_timers_and_animations();
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

#[test]
fn separator_is_a_narrow_noninteractive_line_between_command_buttons() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    main.set_preferences_ready(true);
    main.set_user_commands(
        Rc::new(slint::VecModel::from(vec![
            row(1),
            UserCommandRow {
                separator: true,
                ..row(2)
            },
            row(3),
        ]))
        .into(),
    );
    let calls = Rc::new(RefCell::new(Vec::new()));
    let captured = calls.clone();
    main.on_execute_user_command(move |id, _| captured.borrow_mut().push(id.to_string()));
    main.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620, "command-toolbar-separator");
    assert!(
        modeless::accessible_item(&native, AccessibleRole::Button, "Custom command 2").is_none()
    );
    let separator =
        modeless::accessible_item(&native, AccessibleRole::Text, "Command group separator")
            .unwrap();
    assert!(separator.geometry().width() <= 12.0);
    separator.accessible_action(&i_slint_core::accessibility::AccessibilityAction::Default);
    assert!(calls.borrow().is_empty());
    click(&native, "Custom command 3");
    assert_eq!(*calls.borrow(), ["command-3"]);
}
