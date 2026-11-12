//! Explicit Find actions are visible independently of panel navigation.
#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;
use i_slint_core::{accessibility::AccessibilityAction, items::AccessibleRole};
use midnight_ui::{FindRow, FindWindow};
use slint::{
    ComponentHandle, VecModel,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

#[test]
fn find_exposes_separate_view_edit_and_open_actions() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.invoke_apply_preferences(0, 0, "".into(), 13, 17);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 1000, 620, "find-explicit-actions");
    let actions = Rc::new(RefCell::new(Vec::new()));
    window.on_file_action({
        let actions = actions.clone();
        move |token, action| {
            actions.borrow_mut().push((token.to_string(), action));
        }
    });
    window.set_rows(
        Rc::new(VecModel::from(vec![
            FindRow {
                token: "4:11".into(),
                name: "selected.txt".into(),
                ..Default::default()
            },
            FindRow {
                token: "4:12".into(),
                name: "folder".into(),
                directory: true,
                ..Default::default()
            },
        ]))
        .into(),
    );
    window.set_selected_row(0);
    modeless::draw(&native, 1000, 620, "find-explicit-actions-selected");
    for label in ["View", "Edit", "Open"] {
        assert!(
            modeless::accessible_item(&native, AccessibleRole::Button, label).is_some(),
            "Find is missing its explicit {label} action"
        );
        modeless::accessible_item(&native, AccessibleRole::Button, label)
            .unwrap()
            .accessible_action(&AccessibilityAction::Default);
    }
    assert_eq!(
        &*actions.borrow(),
        &[
            ("4:11".to_owned(), 0),
            ("4:11".to_owned(), 2),
            ("4:11".to_owned(), 3)
        ]
    );
    let keys = Rc::new(RefCell::new(Vec::new()));
    window.on_action_key({
        let keys = keys.clone();
        move |event, token| {
            keys.borrow_mut()
                .push((event.text.to_string(), token.to_string()));
            true
        }
    });
    modeless::accessible_item(
        &native,
        AccessibleRole::ListItem,
        "selected.txt ·  ·  ·  · ",
    )
    .unwrap()
    .accessible_action(&AccessibilityAction::Default);
    native.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::F3.into(),
    });
    assert_eq!(keys.borrow().len(), 1);
    assert_eq!(keys.borrow()[0].1, "4:11");
    window.set_selected_row(1);
    for label in ["View", "Edit"] {
        modeless::accessible_item(&native, AccessibleRole::Button, label)
            .unwrap()
            .accessible_action(&AccessibilityAction::Default);
    }
    assert_eq!(
        actions.borrow().len(),
        3,
        "folder View/Edit buttons must be disabled"
    );
}
