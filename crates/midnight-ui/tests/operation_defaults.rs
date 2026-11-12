//! Confirm defaults through actual child-window keyboard input, without filesystem operations.
#[path = "support/window_input.rs"]
mod input;
#[path = "support/modeless.rs"]
mod modeless;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, FocusReason, FocusScope},
    window::WindowInner,
};
use midnight_ui::OperationWindow;
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc};

fn window() -> (OperationWindow, Rc<MinimalSoftwareWindow>) {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = OperationWindow::new().unwrap();
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 640, 420, "operation-default");
    (window, native)
}

#[test]
fn transfer_enter_proceeds_and_explicit_cancel_and_escape_remain_available() {
    let (window, child) = window();
    let confirmations = Rc::new(RefCell::new(Vec::new()));
    let cancellations = Rc::new(RefCell::new(0));
    let sink = confirmations.clone();
    window.on_confirm(move |path| sink.borrow_mut().push(path));
    let sink = cancellations.clone();
    window.on_request_dismiss(move || *sink.borrow_mut() += 1);
    let path = "D:\\target 文档\\";
    window.set_initial_name(path.into());
    window.set_has_name(true);
    window.set_option_visible(true);
    window.set_option_enabled(true);
    window.set_option_label("Preserve exact modified time".into());
    for title in ["Copy", "Move"] {
        window.set_title_text(title.into());
        window.set_confirm_text(title.into());
        window.invoke_begin();
        confirmations.borrow_mut().clear();
        *cancellations.borrow_mut() = 0;
        input::key(&child, Key::Return);
        assert_eq!(
            confirmations.borrow().as_slice(),
            &[slint::SharedString::from(path)]
        );
        assert_eq!(*cancellations.borrow(), 0);

        // Space toggles the option; Enter retains the same default action.
        confirmations.borrow_mut().clear();
        window.invoke_begin();
        input::key(&child, Key::Tab);
        input::key(&child, Key::Space);
        assert!(confirmations.borrow().is_empty());
        input::key(&child, Key::Return);
        assert_eq!(confirmations.borrow().len(), 1);

        confirmations.borrow_mut().clear();
        window.invoke_begin();
        input::key(&child, Key::Escape);
        assert_eq!(*cancellations.borrow(), 1);
        assert!(confirmations.borrow().is_empty());

        window.invoke_begin();
        input::key(&child, Key::Tab);
        input::key(&child, Key::Tab);
        input::key(&child, Key::Return);
        assert_eq!(*cancellations.borrow(), 2);
        assert!(confirmations.borrow().is_empty());

        window.set_initial_name("".into());
        window.invoke_begin();
        input::key(&child, Key::Return);
        assert!(
            confirmations.borrow().is_empty(),
            "empty destinations never submit"
        );
        window.set_initial_name(path.into());
    }
}

#[test]
fn delete_default_does_not_remove_explicit_cancellation() {
    let (window, child) = window();
    let confirmed = Rc::new(RefCell::new(0));
    let cancelled = Rc::new(RefCell::new(0));
    let sink = confirmed.clone();
    window.on_confirm(move |_| *sink.borrow_mut() += 1);
    let sink = cancelled.clone();
    window.on_request_dismiss(move || *sink.borrow_mut() += 1);
    window.set_has_name(false);
    window.set_title_text("Permanently delete".into());
    window.set_confirm_text("Permanently delete".into());
    window.set_dangerous(true);
    window.invoke_begin();
    input::key(&child, Key::Escape);
    window.invoke_begin();
    modeless::click(&child, "Cancel");
    // Native X is covered through the installed host in native_operation_windows.
    assert_eq!(*cancelled.borrow(), 2);
    assert_eq!(*confirmed.borrow(), 0);
    window.invoke_begin();
    input::key(&child, Key::Return);
    assert_eq!(*confirmed.borrow(), 1);
    assert_eq!(*cancelled.borrow(), 2);
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

#[test]
fn enter_on_accessibility_focused_cancel_never_confirms_delete() {
    let (window, child) = window();
    let confirmed = Rc::new(RefCell::new(0));
    let cancelled = Rc::new(RefCell::new(0));
    let sink = confirmed.clone();
    window.on_confirm(move |_| *sink.borrow_mut() += 1);
    let sink = cancelled.clone();
    window.on_request_dismiss(move || *sink.borrow_mut() += 1);
    window.set_has_name(false);
    for dangerous in [false, true] {
        window.set_dangerous(dangerous);
        window.invoke_begin();
        let cancel = modeless::accessible_item(&child, AccessibleRole::Button, "Cancel").unwrap();
        let scope = focus_scope(cancel).expect("Cancel keyboard focus scope");
        let inner = WindowInner::from_pub(child.window());
        inner.set_focus_item(&scope, true, FocusReason::Programmatic);
        assert_eq!(inner.focus_item.borrow().upgrade().as_ref(), Some(&scope));
        input::key(&child, Key::Return);
        assert_eq!(
            *confirmed.borrow(),
            0,
            "Enter on Cancel never confirms deletion"
        );
        assert_eq!(*cancelled.borrow(), if dangerous { 2 } else { 1 });
    }
}
