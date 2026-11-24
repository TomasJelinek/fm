//! Confirm defaults through actual keyboard input to the shipped embedded operation
//! form, without filesystem operations.
#[path = "support/embedded_form.rs"]
mod form;
#[path = "support/window_input.rs"]
mod input;
use crate::support::modeless;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, FocusReason, FocusScope},
    window::WindowInner,
};
use midnight_ui::OperationPresentation;
use slint::{ComponentHandle, platform::Key};
use std::{cell::RefCell, rc::Rc};

/// The embedded operation form of a bare main window, rendered once; nothing closes
/// it, so each test sees every confirmation and cancellation request.
fn host() -> form::Host {
    let host = form::Host::new();
    host.main
        .global::<OperationPresentation>()
        .set_token("operation".into());
    host.main.global::<OperationPresentation>().invoke_begin();
    host.draw(640, 420, "operation-default");
    host
}

/// (Re)start the form, which restores the initial name and the default focus.
/// Keyboard input needs the laid-out form, not a new frame.
fn begin(host: &form::Host) {
    host.main.global::<OperationPresentation>().invoke_begin();
    host.layout(640, 420);
}

#[test]
fn transfer_enter_proceeds_and_explicit_cancel_and_escape_remain_available() {
    let host = host();
    let child = host.native.clone();
    let form = host.main.global::<OperationPresentation>();
    let confirmations = Rc::new(RefCell::new(Vec::new()));
    let cancellations = Rc::new(RefCell::new(0));
    let sink = confirmations.clone();
    form.on_confirm(move |_, path| sink.borrow_mut().push(path));
    let sink = cancellations.clone();
    form.on_request_dismiss(move |_| *sink.borrow_mut() += 1);
    let path = "D:\\target 文档\\";
    form.set_initial_name(path.into());
    form.set_has_name(true);
    form.set_option_visible(true);
    form.set_option_enabled(true);
    form.set_option_label("Preserve times and attributes".into());
    for title in ["Copy", "Move"] {
        form.set_title_text(title.into());
        form.set_confirm_text(title.into());
        begin(&host);
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
        begin(&host);
        input::key(&child, Key::Tab);
        input::key(&child, Key::Space);
        assert!(confirmations.borrow().is_empty());
        input::key(&child, Key::Return);
        assert_eq!(confirmations.borrow().len(), 1);

        confirmations.borrow_mut().clear();
        begin(&host);
        input::key(&child, Key::Escape);
        assert_eq!(*cancellations.borrow(), 1);
        assert!(confirmations.borrow().is_empty());

        begin(&host);
        input::key(&child, Key::Tab);
        input::key(&child, Key::Tab);
        input::key(&child, Key::Return);
        assert_eq!(*cancellations.borrow(), 2);
        assert!(confirmations.borrow().is_empty());

        form.set_initial_name("".into());
        begin(&host);
        input::key(&child, Key::Return);
        assert!(
            confirmations.borrow().is_empty(),
            "empty destinations never submit"
        );
        form.set_initial_name(path.into());
    }
}

#[test]
fn transfer_wait_option_toggles_without_submitting_and_preserves_default_enter() {
    let host = host();
    let child = host.native.clone();
    let form = host.main.global::<OperationPresentation>();
    let confirmed = Rc::new(RefCell::new(Vec::new()));
    let cancelled = Rc::new(RefCell::new(0));
    let sink = confirmed.clone();
    form.on_confirm(move |_, value| sink.borrow_mut().push(value));
    let sink = cancelled.clone();
    form.on_request_dismiss(move |_| *sink.borrow_mut() += 1);
    form.set_has_name(true);
    form.set_initial_name("D:\\target 文档".into());
    form.set_wait_visible(true);
    form.set_option_visible(true);
    form.set_option_label("Preserve times and attributes".into());
    for title in ["Copy", "Move"] {
        form.set_title_text(title.into());
        form.set_confirm_text(title.into());
        for metadata in [false, true] {
            form.set_option_enabled(metadata);
            form.set_wait_checked(false);
            begin(&host);
            confirmed.borrow_mut().clear();
            input::key(&child, Key::Tab);
            if metadata {
                input::key(&child, Key::Tab);
            }
            input::key(&child, Key::Space);
            assert!(form.get_wait_checked());
            assert!(confirmed.borrow().is_empty());
            input::key(&child, Key::Return);
            assert_eq!(
                confirmed.borrow().as_slice(),
                &[slint::SharedString::from("D:\\target 文档")]
            );
            confirmed.borrow_mut().clear();
            input::key(&child, Key::Tab);
            input::key(&child, Key::Return);
            assert!(
                confirmed.borrow().is_empty(),
                "Enter on Cancel submitted {title}"
            );
        }
    }
    assert_eq!(*cancelled.borrow(), 4);
}

#[test]
fn delete_default_does_not_remove_explicit_cancellation() {
    let host = host();
    let child = host.native.clone();
    let form = host.main.global::<OperationPresentation>();
    let confirmed = Rc::new(RefCell::new(0));
    let cancelled = Rc::new(RefCell::new(0));
    let sink = confirmed.clone();
    form.on_confirm(move |_, _| *sink.borrow_mut() += 1);
    let sink = cancelled.clone();
    form.on_request_dismiss(move |_| *sink.borrow_mut() += 1);
    form.set_has_name(false);
    form.set_title_text("Permanently delete".into());
    form.set_confirm_text("Permanently delete".into());
    form.set_dangerous(true);
    begin(&host);
    input::key(&child, Key::Escape);
    begin(&host);
    host.click("Cancel");
    // The installed host covers native close requests in native_operation_windows.
    assert_eq!(*cancelled.borrow(), 2);
    assert_eq!(*confirmed.borrow(), 0);
    begin(&host);
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
    let host = host();
    let child = host.native.clone();
    let form = host.main.global::<OperationPresentation>();
    let confirmed = Rc::new(RefCell::new(0));
    let cancelled = Rc::new(RefCell::new(0));
    let sink = confirmed.clone();
    form.on_confirm(move |_, _| *sink.borrow_mut() += 1);
    let sink = cancelled.clone();
    form.on_request_dismiss(move |_| *sink.borrow_mut() += 1);
    form.set_has_name(false);
    for dangerous in [false, true] {
        form.set_dangerous(dangerous);
        begin(&host);
        let cancel = host.item(AccessibleRole::Button, "Cancel");
        let scope = focus_scope(cancel).expect("Cancel keyboard focus scope");
        let inner = WindowInner::from_pub(host.main.window());
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
