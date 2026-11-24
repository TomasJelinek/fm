//! `PanelChoices` presentation: Show hidden files and the Select/Deselect by mask
//! dialog. The Presenter owns the choices; this bridge only mirrors and forwards.
use crate::{MainWindow, PanelChoices, Presenter, dialog_focus::restore_after_popup};
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
    let choices = main.global::<PanelChoices>();
    let context = (main.as_weak(), Rc::downgrade(&state), send);
    let (window, presenter, sender) = context.clone();
    choices.on_show_hidden_files_changed(move |show| {
        act(&window, &presenter, None, |p| {
            p.change_show_hidden_files(show, sender.as_ref());
        });
    });
    let (window, presenter, sender) = context.clone();
    choices.on_mask_apply(move |token, pattern, folders| {
        act(&window, &presenter, Some(&token), |p| {
            p.submit_mask_selection(pattern.as_str(), folders, sender.as_ref());
        });
    });
    let (window, presenter, _) = context;
    choices.on_mask_dismiss(move |token| {
        act(
            &window,
            &presenter,
            Some(&token),
            Presenter::cancel_mask_selection,
        );
    });
}

/// Run `action` unless a dialog `token` names an earlier opening, then refresh.
fn act(
    window: &slint::Weak<MainWindow>,
    presenter: &Weak<RefCell<Presenter>>,
    token: Option<&SharedString>,
    action: impl FnOnce(&mut Presenter),
) {
    let (Some(main), Some(state)) = (window.upgrade(), presenter.upgrade()) else {
        return;
    };
    if token.is_some() {
        // The dialog may close: retire a hint its field or buttons still show
        // before the popup drops its owner, and before borrowing the Presenter.
        crate::tooltip::dismiss(main.window());
    }
    {
        let Ok(mut p) = state.try_borrow_mut() else {
            return;
        };
        if let Some(token) = token
            && (!p.mask_dialog.open || token.as_str() != p.mask_dialog.generation.to_string())
        {
            return;
        }
        action(&mut p);
    }
    crate::refresh(&main, &state.borrow());
}

pub(crate) fn refresh(main: &MainWindow, p: &Presenter) {
    let choices = main.global::<PanelChoices>();
    let show = p
        .feed(p.active)
        .map_or(p.preferences.values.show_hidden_files, |feed| {
            feed.projection.desired.show_hidden
        });
    if choices.get_show_hidden_files() != show {
        choices.set_show_hidden_files(show);
    }
    let dialog = &p.mask_dialog;
    let error = SharedString::from(dialog.error.as_str());
    if choices.get_mask_error() != error {
        choices.set_mask_error(error);
    }
    let token = if dialog.open {
        SharedString::from(dialog.generation.to_string())
    } else {
        SharedString::default()
    };
    if choices.get_mask_token() == token {
        return;
    }
    if dialog.open {
        // The dialog reads these when it begins; afterwards its own controls own them.
        choices.set_mask_select(dialog.select);
        choices.set_mask_pattern(dialog.pattern.as_str().into());
        choices.set_mask_folders(dialog.folders);
        choices.set_mask_token(token);
        return;
    }
    choices.set_mask_token(token);
    // Return focus to the file list once the popup has really closed.
    let weak = main.as_weak();
    restore_after_popup(main, None, move || {
        weak.upgrade()
            .is_some_and(|main| main.global::<PanelChoices>().get_mask_token().is_empty())
    });
}
