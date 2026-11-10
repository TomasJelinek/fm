//! Editor form bindings. Native paths are displayed here, never opened here.
use crate::{EditorOptionsDraft, EditorOptionsView, MainWindow, Presenter};
use midnight_core::Command;
use slint::ComponentHandle;
use std::hash::{Hash, Hasher};
use std::{cell::RefCell, rc::Rc};

pub(crate) fn refresh(window: &MainWindow, p: &Presenter) {
    let view = window.global::<EditorOptionsView>();
    let mut configuration = std::collections::hash_map::DefaultHasher::new();
    p.preferences
        .values
        .editor
        .executable
        .hash(&mut configuration);
    p.preferences
        .values
        .editor
        .arguments
        .hash(&mut configuration);
    let revision = format!(
        "{}:{}:{}:{}",
        p.editor.revision,
        p.preferences.revision.0,
        p.editor_draft_loaded(),
        configuration.finish()
    );
    view.set_dirty(p.editor.dirty);
    view.set_busy(p.settings_picker.busy());
    view.set_error(p.editor.error.as_str().into());
    view.set_default_label(p.editor.default_label.as_str().into());
    if view.get_revision() == revision {
        return;
    }
    let draft = p.editor_draft();
    view.set_draft(EditorOptionsDraft {
        executable: draft.executable.into(),
        arguments: String::new().into(), // Old compatibility field is never rendered.
        use_default: draft.use_default,
    });
    view.set_argument_rows(crate::argument_rows::sync(
        view.get_argument_rows(),
        &draft.arguments,
    ));
    view.set_revision(revision.into());
}

pub(crate) fn install(
    window: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let weak = window.as_weak();
    let p = state.clone();
    window.global::<EditorOptionsView>().on_edit(move |form| {
        p.borrow_mut()
            .edit_editor_draft(form.executable.to_string(), form.use_default);
        if let Some(w) = weak.upgrade() {
            refresh(&w, &p.borrow());
        }
    });
    let weak = window.as_weak();
    let p = state.clone();
    window
        .global::<EditorOptionsView>()
        .on_argument(move |operation, row, text| {
            p.borrow_mut()
                .edit_editor_argument(operation, row, text.as_str());
            if let Some(w) = weak.upgrade() {
                refresh(&w, &p.borrow());
            }
        });
    let weak = window.as_weak();
    let p = state.clone();
    let sender = send.clone();
    window.global::<EditorOptionsView>().on_browse(move || {
        if let Some(w) = weak.upgrade() {
            let owner = crate::picker_window::owner(&w);
            p.borrow_mut().request_settings_picker(
                super::presenter::settings_picker::Target::Editor,
                owner,
                sender.as_ref(),
            );
            crate::refresh(&w, &p.borrow());
            crate::settings_picker_presentation::wake(&w, &p, sender.clone());
        }
    });
    let weak = window.as_weak();
    let discarded = state.clone();
    window.global::<EditorOptionsView>().on_discard(move || {
        discarded.borrow_mut().discard_editor_draft();
        if let Some(w) = weak.upgrade() {
            crate::refresh(&w, &discarded.borrow());
        }
    });
    let weak = window.as_weak();
    window.global::<EditorOptionsView>().on_save(move || {
        state.borrow_mut().save_editor_draft(send.as_ref());
        if let Some(w) = weak.upgrade() {
            crate::refresh(&w, &state.borrow());
        }
    });
}
