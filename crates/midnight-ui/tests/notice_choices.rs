//! A notice that offers actions (an archive to close when every one that may be open is
//! open) shows each as a labelled button that Tab reaches and Enter presses; Escape and
//! Cancel choose nothing.
#[path = "support/embedded_form.rs"]
mod form;
use crate::support::modeless;

use i_slint_core::{
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::AccessibleRole,
    window::WindowInner,
};
use midnight_ui::NoticePresentation;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, platform::Key};
use std::{cell::RefCell, rc::Rc};

const RIGHT: &str = "Close x.7z (right panel)";
const TAB: &str = "Close sample.zip (left panel, tab 2 and a viewer)";

/// An offer with two archives to close; what it reports is recorded.
fn offer() -> (form::Host, Rc<RefCell<Vec<String>>>) {
    let host = form::Host::new();
    let answers = Rc::new(RefCell::new(Vec::new()));
    let notice = host.main.global::<NoticePresentation>();
    let chosen = answers.clone();
    let weak = host.main.as_weak();
    notice.on_choose(move |token, index| {
        chosen.borrow_mut().push(format!("{token}:{index}"));
        if let Some(main) = weak.upgrade() {
            main.global::<NoticePresentation>().set_token("".into());
        }
    });
    let dismissed = answers.clone();
    let weak = host.main.as_weak();
    notice.on_dismiss(move |token| {
        dismissed.borrow_mut().push(format!("{token}:dismissed"));
        if let Some(main) = weak.upgrade() {
            main.global::<NoticePresentation>().set_token("".into());
        }
    });
    notice.set_title("Two archives are already open".into());
    notice.set_error(false);
    notice.set_body("Close one of them to open b.7z.".into());
    let choices: Vec<SharedString> = vec![RIGHT.into(), TAB.into()];
    notice.set_choices(ModelRc::new(VecModel::from(choices)));
    notice.set_cancel(true);
    notice.set_generation(1);
    notice.set_token("offer".into());
    host.draw(1000, 700, "notice-choices");
    (host, answers)
}

/// Whether the keyboard focus is `item` or inside it.
fn focused(host: &form::Host, item: &ItemRc) -> bool {
    let inner = WindowInner::from_pub(host.main.window());
    let mut focus = inner.focus_item.borrow().upgrade();
    while let Some(current) = focus {
        if &current == item {
            return true;
        }
        focus = current.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    false
}

#[test]
fn each_choice_is_a_labelled_button_and_close_cancels() {
    let (host, answers) = offer();
    for label in [RIGHT, TAB, "Cancel"] {
        let button = host.visible_item(AccessibleRole::Button, label);
        form::assert_inside(&button, label);
    }
    assert!(
        host.try_item(AccessibleRole::Button, "Close").is_none(),
        "the dialog's own Close button says Cancel"
    );
    host.click(TAB);
    assert_eq!(*answers.borrow(), ["offer:1"]);
    assert!(!host.is_open());
}

#[test]
fn tab_reaches_a_choice_and_enter_presses_it() {
    let (host, answers) = offer();
    let cancel = host.visible_item(AccessibleRole::Button, "Cancel");
    assert!(focused(&host, &cancel), "the dialog opens on Cancel");
    let target = host.visible_item(AccessibleRole::Button, RIGHT);
    let mut reached = false;
    for _ in 0..12 {
        host.key(Key::Tab);
        if focused(&host, &target) {
            reached = true;
            break;
        }
    }
    assert!(reached, "Tab reaches every choice");
    assert!(
        answers.borrow().is_empty(),
        "moving the focus chooses nothing"
    );
    host.key(Key::Return);
    assert_eq!(*answers.borrow(), ["offer:0"]);
}

#[test]
fn escape_chooses_nothing() {
    let (host, answers) = offer();
    host.key(Key::Escape);
    assert_eq!(*answers.borrow(), ["offer:dismissed"]);
    assert!(!host.is_open());
}

#[test]
fn cancel_chooses_nothing() {
    let (host, answers) = offer();
    host.click("Cancel");
    assert_eq!(*answers.borrow(), ["offer:dismissed"]);
    assert!(!host.is_open());
}
