//! The archive password dialog through the real window: typing, Enter, Escape, Show
//! password and the wrong-password message, at normal and minimum sizes.
use crate::support::ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::ArchivePasswords;
use slint::{ComponentHandle, SharedString, platform::Key};
use std::{cell::RefCell, rc::Rc};

type Calls = Rc<RefCell<Vec<(String, String)>>>;

/// Open the dialog as the Presenter's bridge does, recording its callbacks.
fn open(ui: &ui::TestUi) -> (Calls, Rc<RefCell<Vec<String>>>) {
    let dialog = ui.window.global::<ArchivePasswords>();
    let submitted: Calls = Rc::default();
    let dismissed: Rc<RefCell<Vec<String>>> = Rc::default();
    let record = submitted.clone();
    dialog.on_submit(move |token, password| {
        record
            .borrow_mut()
            .push((token.to_string(), password.to_string()));
    });
    let record = dismissed.clone();
    dialog.on_dismiss(move |token| record.borrow_mut().push(token.to_string()));
    dialog.set_archive("share.zip".into());
    dialog.set_purpose("Enter its password to view balcony.emc.".into());
    dialog.set_token("7".into());
    ui::settle_layout(ui.window.window());
    (submitted, dismissed)
}

fn type_text(ui: &ui::TestUi, text: &str) {
    for character in text.chars() {
        ui::key(&ui.window, SharedString::from(character.to_string()));
    }
}

#[test]
fn typing_and_enter_submit_the_password_and_escape_cancels() {
    let ui = ui::TestUi::new();
    let (submitted, dismissed) = open(&ui);
    assert!(ui::has_popup(&ui.window), "the dialog is a modal popup");
    ui::popup_item(&ui.window, "Password", AccessibleRole::TextInput);
    type_text(&ui, "héslo 🔑");
    ui::key(&ui.window, Key::Return);
    assert_eq!(
        submitted.borrow().as_slice(),
        [("7".to_owned(), "héslo 🔑".to_owned())]
    );
    ui::key(&ui.window, Key::Escape);
    assert_eq!(dismissed.borrow().as_slice(), ["7"]);
}

#[test]
fn a_wrong_password_shows_why_and_ok_waits_while_one_is_checked() {
    let ui = ui::TestUi::new();
    let (submitted, _) = open(&ui);
    let dialog = ui.window.global::<ArchivePasswords>();
    dialog.set_busy(true);
    type_text(&ui, "x");
    ui::key(&ui.window, Key::Return);
    assert!(submitted.borrow().is_empty(), "Enter waits for the answer");
    dialog.set_busy(false);
    dialog.set_error("Wrong password — try again.".into());
    dialog.set_attempt(1);
    ui::settle_layout(ui.window.window());
    assert!(
        ui::try_popup_item(
            &ui.window,
            "Wrong password — try again.",
            AccessibleRole::Text
        )
        .is_some(),
        "the reason is shown under the field"
    );
    // The refused text is selected: typing replaces it.
    type_text(&ui, "right");
    ui::key(&ui.window, Key::Return);
    assert_eq!(
        submitted
            .borrow()
            .last()
            .map(|(_, password)| password.as_str()),
        Some("right")
    );
}

#[test]
fn show_password_and_ok_work_by_pointer_and_the_dialog_fits_the_smallest_window() {
    let mut ui = ui::TestUi::new();
    ui.set_size(320, 200);
    let (submitted, dismissed) = open(&ui);
    type_text(&ui, "abc");
    ui::click_popup(&ui.window, "Show password", AccessibleRole::Checkbox);
    ui::click_popup(&ui.window, "OK", AccessibleRole::Button);
    assert_eq!(
        submitted.borrow().as_slice(),
        [("7".to_owned(), "abc".to_owned())]
    );
    // Dismissal stays reachable at 320 × 200.
    ui::click_popup(&ui.window, "Cancel", AccessibleRole::Button);
    assert_eq!(dismissed.borrow().as_slice(), ["7"]);
}
