//! Options footer Export/Import configuration bindings: input and presentation only.
//! Native choosers, file access and validation run in backend services.
use crate::{MainWindow, Presenter, SettingsFileView};
use midnight_core::Command;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

pub(crate) fn refresh(window: &MainWindow, p: &Presenter) {
    let view = window.global::<SettingsFileView>();
    let state = &p.preferences.settings_file;
    view.set_busy(p.settings_picker.busy() || state.busy());
    view.set_importing(state.importing());
    let message = crate::shown_text(&state.message);
    if view.get_message() != message {
        view.set_message(message);
    }
    view.set_error(state.error);
    view.set_replace_pending(state.replace_pending());
}

pub(crate) fn install(
    window: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let view = window.global::<SettingsFileView>();
    for export in [true, false] {
        let weak = window.as_weak();
        let p = state.clone();
        let sender = send.clone();
        let choose = move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let owner = crate::picker_window::owner(&w);
            if export {
                p.borrow_mut().begin_settings_export(owner, sender.as_ref());
            } else {
                p.borrow_mut().begin_settings_import(owner, sender.as_ref());
            }
            crate::refresh(&w, &p.borrow());
            // Closing Options cancels an open chooser; this retries a refused cancellation.
            crate::settings_picker_presentation::wake(&w, &p, sender.clone());
        };
        if export {
            view.on_export_file(choose);
        } else {
            view.on_import_file(choose);
        }
    }
    let weak = window.as_weak();
    let p = state.clone();
    view.on_replace(move || {
        let Some(w) = weak.upgrade() else {
            return;
        };
        p.borrow_mut().replace_settings_export(send.as_ref());
        crate::refresh(&w, &p.borrow());
    });
    let weak = window.as_weak();
    view.on_keep(move || {
        let Some(w) = weak.upgrade() else {
            return;
        };
        state.borrow_mut().keep_settings_export();
        crate::refresh(&w, &state.borrow());
    });
}
