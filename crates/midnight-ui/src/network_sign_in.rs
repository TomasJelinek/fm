//! In-app sign-in for a single network navigation challenge.
use crate::{MainWindow, NetworkSignIn, Presenter, dialog_focus::restore_after_popup, shown_text};
use midnight_core::Command;
use slint::{ComponentHandle, SharedString};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

pub(crate) fn install(
    main: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let dialog = main.global::<NetworkSignIn>();
    let (window, presenter) = (main.as_weak(), Rc::downgrade(&state));
    dialog.on_submit(move |token, username, domain, password| {
        act(&window, &presenter, |p| {
            p.submit_network_sign_in(
                token.as_str(),
                username.to_string(),
                domain.to_string(),
                password.to_string(),
                send.as_ref(),
            );
        });
    });
    let (window, presenter) = (main.as_weak(), Rc::downgrade(&state));
    dialog.on_dismiss(move |token| {
        act(&window, &presenter, |p| {
            p.cancel_network_sign_in(token.as_str())
        });
    });
}

fn act(
    window: &slint::Weak<MainWindow>,
    presenter: &Weak<RefCell<Presenter>>,
    action: impl FnOnce(&mut Presenter),
) {
    let (Some(main), Some(state)) = (window.upgrade(), presenter.upgrade()) else {
        return;
    };
    crate::tooltip::dismiss(main.window());
    {
        let Ok(mut p) = state.try_borrow_mut() else {
            return;
        };
        action(&mut p);
    }
    crate::refresh(&main, &state.borrow());
}

pub(crate) fn refresh(main: &MainWindow, p: &Presenter) {
    let dialog = main.global::<NetworkSignIn>();
    let token = dialog.get_token();
    if let Some(prompt) = p.network_sign_in_prompt() {
        let next = SharedString::from(prompt.token().as_str());
        let error = SharedString::from(prompt.error.as_str());
        if dialog.get_error() != error {
            dialog.set_error(error);
        }
        if token != next {
            dialog.set_device(shown_text(&prompt.address.display));
            dialog.set_token(next);
        }
    } else if !token.is_empty() {
        dialog.set_token(SharedString::default());
        let weak = main.as_weak();
        restore_after_popup(main, None, move || {
            weak.upgrade()
                .is_some_and(|main| main.global::<NetworkSignIn>().get_token().is_empty())
        });
    }
}
