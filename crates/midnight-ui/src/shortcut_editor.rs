//! Global view adapter for the searchable shortcut editor and its capture field.
use crate::{KeyboardEditorView, MainWindow, Presenter, ShortcutCommandRow, shortcut_input};
use midnight_core::{Command, ShortcutChord, shortcut_catalog};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

pub(crate) fn capture(window: &MainWindow, event: &i_slint_core::input::KeyEvent) -> bool {
    let view = window.global::<KeyboardEditorView>();
    if !view.get_capturing() {
        return false;
    }
    // The focused capture control records these keys and then moves focus itself.
    if [slint::platform::Key::Tab, slint::platform::Key::Escape]
        .into_iter()
        .any(|key| event.text == slint::SharedString::from(key))
    {
        return false;
    }
    if window.get_native_alt_gr() || window.get_native_digit_event() {
        return true;
    }
    view.invoke_capture_key(event.clone());
    true
}

pub(crate) fn capture_physical(window: &MainWindow, chord: &ShortcutChord) -> bool {
    let view = window.global::<KeyboardEditorView>();
    if !view.get_capturing() {
        return false;
    }
    if !window.get_native_alt_gr() {
        view.invoke_capture_chord(chord.display().into());
    }
    true
}

pub(crate) fn install(
    window: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    macro_rules! bind {
        ($name:ident, |$w:ident, $p:ident, $sender:ident $(, $arg:ident)*| $body:block) => {{
            let weak = window.as_weak();
            let state = state.clone();
            let sender = send.clone();
            window.global::<KeyboardEditorView>().$name(move |$($arg),*| {
                let Some($w) = weak.upgrade() else { return Default::default(); };
                let result = {
                    let mut state = state.borrow_mut();
                    let $p = &mut *state;
                    let $sender = sender.as_ref();
                    $body
                };
                crate::refresh(&$w, &state.borrow());
                result
            });
        }};
    }
    bind!(on_search, |_w, p, _send, query| {
        p.filter_shortcut_commands(query.to_string());
    });
    bind!(on_select, |_w, p, _send, id| {
        if let Some(index) = shortcut_catalog()
            .iter()
            .position(|action| action.id == id.as_str())
        {
            p.select_shortcut(index as i32);
        }
    });
    bind!(on_move_selection, |_w, p, _send, step| {
        p.move_shortcut_command(step);
    });
    bind!(on_capture_key, |window, p, _send, event| {
        if !window.get_native_alt_gr()
            && !window.get_native_digit_event()
            && let Some(chord) = shortcut_input::chord(&event)
        {
            p.capture_editor_shortcut(chord);
        }
        true
    });
    bind!(on_capture_chord, |_w, p, _send, text| {
        if let Ok(chord) = ShortcutChord::parse(text.as_str()) {
            p.capture_editor_shortcut(chord);
        }
    });
    bind!(on_clear, |_w, p, _send| {
        p.clear_editor_shortcut();
    });
    bind!(on_assign, |_w, p, send, id, candidate| {
        p.assign_editor_shortcut(id.as_str(), candidate.as_str(), send);
    });
    bind!(on_remove, |_w, p, send, id, key| {
        p.remove_editor_shortcut(id.as_str(), key.as_str(), send);
    });
    bind!(on_restore, |_w, p, send, id| {
        p.restore_shortcut(id.as_str(), send);
    });
    bind!(on_reset_all, |_w, p, send| {
        p.restore_shortcuts(send);
    });
    refresh(window, &state.borrow());
}

pub(crate) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let view = window.global::<KeyboardEditorView>();
    let state = &presenter.keyboard;
    let action = &shortcut_catalog()[state.selected];
    let revision = format!(
        "{}:{}:{}",
        state.editor_revision_text, action.id, state.error
    );
    if view.get_revision() == revision {
        return;
    }
    let inspection = presenter.inspect_editor_shortcut();
    view.set_selected_id(action.id.as_str().into());
    view.set_selected_label(action.label.as_str().into());
    view.set_filter(state.editor.filter.as_str().into());
    view.set_candidate(
        state
            .editor
            .candidate
            .as_ref()
            .map(ShortcutChord::display)
            .unwrap_or_default()
            .into(),
    );
    view.set_scope(inspection.scope.into());
    view.set_used_by(inspection.used_by.into());
    view.set_issue(if state.error.is_empty() {
        inspection.issue.into()
    } else {
        state.error.as_str().into()
    });
    view.set_can_assign(inspection.can_assign);
    crate::update_strings(
        view.get_aliases(),
        state
            .snapshot
            .map
            .bindings(&action.id)
            .expect("catalog action")
            .iter()
            .map(|chord| chord.display().into())
            .collect(),
        |model| view.set_aliases(model),
    );
    let command_revision = format!(
        "{}:{}:{}",
        state.snapshot.revision, action.id, state.editor.filter
    );
    if view.get_command_revision() != command_revision {
        let rows = shortcut_catalog()
            .iter()
            .filter(|command| {
                crate::presenter::keyboard_editor::matches_filter(command, &state.editor.filter)
            })
            .map(|command| ShortcutCommandRow {
                id: command.id.as_str().into(),
                label: command.label.as_str().into(),
                aliases: state.snapshot.hint_owned(&command.id, true),
                selected: command.id == action.id,
            })
            .collect::<Vec<_>>();
        view.set_selected_row(
            rows.iter()
                .position(|row| row.selected)
                .map_or(-1, |index| index as i32),
        );
        view.set_commands(ModelRc::new(VecModel::from(rows)));
        view.set_command_revision(command_revision.into());
    }
    view.set_revision(revision.into());
}
