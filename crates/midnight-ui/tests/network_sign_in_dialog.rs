use crate::support::ui;
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_ui::NetworkSignIn;
use slint::{ComponentHandle, SharedString, platform::Key};
use std::{cell::RefCell, rc::Rc};

#[test]
fn network_credentials_are_masked_and_cleared_after_submission() {
    let mut ui = ui::TestUi::new();
    ui.set_size(320, 200);
    let dialog = ui.window.global::<NetworkSignIn>();
    let sent = Rc::new(RefCell::new(Vec::new()));
    let record = sent.clone();
    dialog.on_submit(move |token, username, domain, password| {
        record
            .borrow_mut()
            .push((token, username, domain, password));
    });
    let canceled = Rc::new(RefCell::new(None));
    let record = canceled.clone();
    dialog.on_dismiss(move |token| {
        record.replace(Some(token));
    });
    dialog.set_device("Test NAS".into());
    dialog.set_token("0:9".into());
    ui.layout();
    assert!(ui.window.get_embedded_modal_open());

    let type_text = |text: &str| {
        for ch in text.chars() {
            ui::key(&ui.window, SharedString::from(ch.to_string()));
        }
    };
    type_text("uživatel");
    ui::key(&ui.window, Key::Return);
    type_text("test password");
    let password = ui::popup_item(&ui.window, "Password", AccessibleRole::TextInput);
    assert!(
        password
            .accessible_string_property(AccessibleStringProperty::Value)
            .is_none_or(|value| value.is_empty()),
        "accessibility must not reveal the password"
    );
    ui::chord(&ui.window, &[Key::Control], "a");
    ui::chord(&ui.window, &[Key::Control], "x");
    ui::key(&ui.window, Key::Tab);
    type_text("WORKGROUP");
    ui::key(&ui.window, Key::Return);
    {
        let sent = sent.borrow();
        let (token, username, domain, password) = &sent[0];
        assert_eq!(token.as_str(), "0:9");
        assert_eq!(username.as_str(), "uživatel");
        assert_eq!(domain.as_str(), "WORKGROUP");
        assert_eq!(password.as_str(), "test password");
    }

    // The presenter may keep the popup open if the queue is full.
    ui::key(&ui.window, Key::Return);
    assert!(sent.borrow()[1].3.is_empty());
    ui::click_popup(&ui.window, "Cancel", AccessibleRole::Button);
    assert_eq!(canceled.borrow().as_deref(), Some("0:9"));
}
