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
    for rule in &p.preferences.values.editor.rules {
        rule.enabled.hash(&mut configuration);
        (rule.mode as u8).hash(&mut configuration);
        rule.expression.hash(&mut configuration);
        rule.case_sensitive.hash(&mut configuration);
        rule.executable.hash(&mut configuration);
        rule.arguments.hash(&mut configuration);
        rule.working_directory.hash(&mut configuration);
    }
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
    let form = draft.selected_form();
    view.set_selected(draft.selected.unwrap_or(draft.rules.len()) as i32);
    view.set_rule_count(draft.rules.len() as i32);
    let mut labels = draft
        .rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            let program = if rule.use_default {
                "System editor"
            } else {
                rule.executable.as_str()
            };
            format!(
                "{}. {}{} → {}",
                index + 1,
                if rule.enabled { "" } else { "[Off] " },
                rule.expression,
                program
            )
        })
        .collect::<Vec<_>>();
    labels.push(format!(
        "Fallback → {}",
        if draft.use_default {
            "System editor"
        } else {
            draft.executable.as_str()
        }
    ));
    view.set_labels(crate::argument_rows::sync(view.get_labels(), &labels));
    view.set_draft(EditorOptionsDraft {
        executable: form.executable.into(),
        arguments: String::new().into(), // Old compatibility field is never rendered.
        use_default: form.use_default,
        enabled: form.enabled,
        mode: i32::from(form.mode == midnight_core::FilterMode::Regex),
        expression: form.expression.into(),
        case_sensitive: form.case_sensitive,
        working_directory: form.working_directory.into(),
    });
    view.set_argument_rows(crate::argument_rows::sync(
        view.get_argument_rows(),
        &form.arguments,
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
        p.borrow_mut().edit_editor_form(form);
        if let Some(w) = weak.upgrade() {
            refresh(&w, &p.borrow());
        }
    });
    let weak = window.as_weak();
    let p = state.clone();
    window.global::<EditorOptionsView>().on_select(move |row| {
        p.borrow_mut().select_editor_rule(row);
        if let Some(w) = weak.upgrade() {
            refresh(&w, &p.borrow());
        }
    });
    let weak = window.as_weak();
    let p = state.clone();
    window
        .global::<EditorOptionsView>()
        .on_action(move |action| {
            p.borrow_mut().editor_rule_action(action);
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
    let p = state.clone();
    let sender = send.clone();
    window
        .global::<EditorOptionsView>()
        .on_browse_directory(move || {
            if let Some(w) = weak.upgrade() {
                let owner = crate::picker_window::owner(&w);
                p.borrow_mut().request_settings_picker(
                    super::presenter::settings_picker::Target::EditorWorkingDirectory,
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
