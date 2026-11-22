//! `ArchivePasswords` presentation: the archive password dialog. The Presenter owns the
//! request; this bridge mirrors its state and forwards the dialog's callbacks. The typed
//! password passes straight from the callback into the unlock command.
use crate::{ArchivePasswords, MainWindow, Presenter, dialog_focus::restore_after_popup};
use midnight_core::{Command, PaneId};
use slint::{ComponentHandle, SharedString};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    time::Duration,
};

pub(crate) fn install(
    main: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let dialog = main.global::<ArchivePasswords>();
    let (window, presenter) = (main.as_weak(), Rc::downgrade(&state));
    dialog.on_submit(move |token, password| {
        // The callback's text becomes a zeroizing password at once; it goes into the
        // command, or is wiped here when the dialog is stale.
        let password = midnight_core::ArchivePassword::new(password.as_str().to_owned());
        act(&window, &presenter, &token, |p| {
            p.submit_archive_password(password, send.as_ref());
        });
    });
    let (window, presenter) = (main.as_weak(), Rc::downgrade(&state));
    dialog.on_dismiss(move |token| {
        act(&window, &presenter, &token, |p| p.cancel_archive_password());
    });
}

/// Run `action` unless `token` names an earlier opening, then refresh.
fn act(
    window: &slint::Weak<MainWindow>,
    presenter: &Weak<RefCell<Presenter>>,
    token: &SharedString,
    action: impl FnOnce(&mut Presenter),
) {
    let (Some(main), Some(state)) = (window.upgrade(), presenter.upgrade()) else {
        return;
    };
    // The dialog may close: retire a hint its field or buttons still show first.
    crate::tooltip::dismiss(main.window());
    {
        let Ok(mut p) = state.try_borrow_mut() else {
            return;
        };
        let dialog = &p.archive_password;
        if !dialog.open || token.as_str() != dialog.generation.to_string() {
            return;
        }
        action(&mut p);
    }
    crate::refresh(&main, &state.borrow());
}

pub(crate) fn refresh(main: &MainWindow, p: &Presenter) {
    let dialog = main.global::<ArchivePasswords>();
    let state = &p.archive_password;
    let error = SharedString::from(state.error.as_str());
    if dialog.get_error() != error {
        dialog.set_error(error);
    }
    if dialog.get_attempt() != state.attempt {
        dialog.set_attempt(state.attempt);
    }
    let busy = state.pending.is_some();
    if dialog.get_busy() != busy {
        dialog.set_busy(busy);
    }
    // An unlocked archive's viewer opens once the dialog has closed; the viewer
    // callback borrows the Presenter, so it runs after this refresh returns.
    if let Some((pane, basic)) = p.take_archive_view() {
        let weak = main.as_weak();
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(main) = weak.upgrade() {
                main.invoke_view_file(i32::from(pane == PaneId::Right), basic);
            }
        });
    }
    let token = if state.open {
        SharedString::from(state.generation.to_string())
    } else {
        SharedString::default()
    };
    if dialog.get_token() == token {
        return;
    }
    if state.open {
        // The dialog reads these when it begins.
        dialog.set_archive(state.archive.as_str().into());
        dialog.set_purpose(state.purpose.as_str().into());
        dialog.set_token(token);
        return;
    }
    dialog.set_token(token);
    // Return focus to the file list once the popup has really closed.
    let weak = main.as_weak();
    restore_after_popup(main, None, move || {
        weak.upgrade()
            .is_some_and(|main| main.global::<ArchivePasswords>().get_token().is_empty())
    });
}
