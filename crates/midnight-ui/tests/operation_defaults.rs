//! Confirm defaults through actual keyboard input, without filesystem operations.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, FocusReason, FocusScope},
    window::WindowInner,
};
use slint::{ComponentHandle, platform::Key};
use std::{cell::RefCell, rc::Rc};

#[test]
fn transfer_enter_proceeds_and_explicit_cancel_and_escape_remain_available() {
    let mut ui = ui::TestUi::new();
    let confirmations = Rc::new(RefCell::new(Vec::new()));
    let cancellations = Rc::new(RefCell::new(Vec::new()));
    let sink = confirmations.clone();
    ui.window.on_confirm_copy_draft(move |token, path| {
        sink.borrow_mut().push((token, path));
    });
    let sink = cancellations.clone();
    ui.window.on_cancel_copy_draft(move |token| {
        sink.borrow_mut().push(token);
    });
    let path = "D:\\target 文档\\";
    ui.window.set_copy_draft_destination(path.into());
    ui.window.set_copy_draft_metadata_enabled(true);
    for moving in [false, true] {
        ui.window.set_copy_draft_move(moving);
        ui.window.set_copy_draft_token("default-action".into());
        ui.window.invoke_show_copy_draft();
        ui.draw();
        confirmations.borrow_mut().clear();
        cancellations.borrow_mut().clear();
        assert!(confirmations.borrow().is_empty());
        ui::key(&ui.window, Key::Return);
        assert_eq!(
            confirmations.borrow().as_slice(),
            &[("default-action".into(), path.into())]
        );
        assert!(cancellations.borrow().is_empty());

        // A checkbox does not replace the dialog's default action. Space toggles
        // the option; Enter proceeds with the same literal destination.
        confirmations.borrow_mut().clear();
        ui.window.invoke_show_copy_draft();
        ui::key(&ui.window, Key::Tab);
        ui::key(&ui.window, Key::Space);
        assert!(confirmations.borrow().is_empty());
        ui::key(&ui.window, Key::Return);
        assert_eq!(confirmations.borrow().len(), 1);

        confirmations.borrow_mut().clear();
        ui.window.invoke_show_copy_draft();
        ui::key(&ui.window, Key::Escape);
        assert_eq!(cancellations.borrow().len(), 1);
        assert!(confirmations.borrow().is_empty());

        ui.window.invoke_show_copy_draft();
        ui::key(&ui.window, Key::Tab);
        ui::key(&ui.window, Key::Tab);
        ui::key(&ui.window, Key::Return);
        assert_eq!(cancellations.borrow().len(), 2);
        assert!(confirmations.borrow().is_empty());

        ui.window.set_copy_draft_destination("".into());
        ui.window.invoke_show_copy_draft();
        ui::key(&ui.window, Key::Return);
        assert!(
            confirmations.borrow().is_empty(),
            "empty destinations never submit"
        );
        ui.window.set_copy_draft_token("".into());
        ui.window.set_copy_draft_destination(path.into());
    }
}

#[test]
fn delete_default_does_not_remove_explicit_cancellation() {
    let mut ui = ui::TestUi::new();
    let confirmed = Rc::new(RefCell::new(Vec::new()));
    let cancelled = Rc::new(RefCell::new(Vec::new()));
    let sink = confirmed.clone();
    ui.window
        .on_confirm_operation(move |token, _| sink.borrow_mut().push(token));
    let sink = cancelled.clone();
    ui.window
        .on_cancel_operation(move |token| sink.borrow_mut().push(token));
    ui.window.set_operation_has_name(false);
    ui.window.set_operation_title("Permanently delete".into());
    ui.window.set_operation_dangerous(true);
    ui.window.set_operation_token("delete-1".into());
    ui.window.invoke_begin_operation();
    ui.draw();
    assert!(confirmed.borrow().is_empty());
    ui::key(&ui.window, Key::Escape);
    ui.window.invoke_begin_operation();
    ui::click_root(&ui.window, "Cancel", AccessibleRole::Button);
    ui.window.invoke_begin_operation();
    ui::click_root(&ui.window, "Cancel dialog", AccessibleRole::Button);
    assert_eq!(cancelled.borrow().len(), 3);
    assert!(confirmed.borrow().is_empty());
    ui.window.invoke_begin_operation();
    ui::key(&ui.window, Key::Return);
    assert_eq!(
        confirmed.borrow().as_slice(),
        &[slint::SharedString::from("delete-1")]
    );
    assert_eq!(cancelled.borrow().len(), 3);
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
fn enter_on_focused_header_close_cancels_instead_of_confirming_delete() {
    let mut ui = ui::TestUi::new();
    let confirmed = Rc::new(RefCell::new(Vec::new()));
    let cancelled = Rc::new(RefCell::new(Vec::new()));
    let sink = confirmed.clone();
    ui.window
        .on_confirm_operation(move |token, _| sink.borrow_mut().push(token));
    let sink = cancelled.clone();
    ui.window
        .on_cancel_operation(move |token| sink.borrow_mut().push(token));
    ui.window.set_operation_has_name(false);
    ui.window.set_operation_token("focused-close".into());
    for dangerous in [false, true] {
        ui.window.set_operation_dangerous(dangerous);
        ui.window.invoke_begin_operation();
        ui.draw();
        let close = ui::root_item(&ui.window, "Cancel dialog", AccessibleRole::Button);
        let scope = focus_scope(close).expect("close button keyboard focus scope");
        let inner = WindowInner::from_pub(ui.window.window());
        // Exercise the same focus route an accessibility client can request.
        // Calling accessible-action-default would bypass key capture entirely.
        inner.set_focus_item(&scope, true, FocusReason::Programmatic);
        assert_eq!(inner.focus_item.borrow().upgrade().as_ref(), Some(&scope));
        ui::key(&ui.window, Key::Return);
        assert!(
            confirmed.borrow().is_empty(),
            "Enter on the focused close button must never confirm deletion"
        );
        assert_eq!(cancelled.borrow().len(), if dangerous { 2 } else { 1 });
    }
}
