//! Shared routing and cached presentation for standalone and presenter-backed UI.
use crate::{
    MainWindow, Presenter, ShortcutHints, file_key_bridge, file_keys,
    presenter::keyboard::{KeyboardBindings, KeyboardState},
    refresh, shortcut_input,
};
use midnight_core::{Command, ShortcutChord, ShortcutContext, shortcut_catalog};
use slint::{ComponentHandle, Model, VecModel};
use std::{cell::RefCell, rc::Rc, sync::Arc};

type Bindings = Rc<RefCell<Arc<KeyboardBindings>>>;

pub(crate) fn install_default(window: &MainWindow) {
    install_routing(window, KeyboardState::default().bindings);
}

fn context(window: &MainWindow) -> (ShortcutContext, i32, bool) {
    let pane = window.get_focused_pane();
    if pane >= 0 {
        let (active, composing) = if pane == 0 {
            (
                window.get_left_search_active(),
                !window.get_left_search_preedit().is_empty(),
            )
        } else {
            (
                window.get_right_search_active(),
                !window.get_right_search_preedit().is_empty(),
            )
        };
        return (
            if active {
                ShortcutContext::Search
            } else {
                ShortcutContext::Files
            },
            pane,
            composing,
        );
    }
    if window.get_volume_popup_pane() >= 0 {
        (
            ShortcutContext::Volume,
            window.get_volume_popup_pane(),
            false,
        )
    } else if window.get_command_focused() {
        (ShortcutContext::Command, window.get_active_pane(), false)
    } else {
        (ShortcutContext::Other, window.get_active_pane(), false)
    }
}

fn resolve(
    bindings: &Bindings,
    chord: &ShortcutChord,
    context: ShortcutContext,
) -> Option<file_keys::FileKey> {
    let snapshot = bindings.borrow().clone();
    let action = snapshot.map.resolve(chord, context)?;
    file_keys::semantic(&action.id, context == ShortcutContext::Search)
}

fn logical_digit_is_native(window: &MainWindow, chord: &ShortcutChord) -> bool {
    window.get_native_digit_event()
        || (window.get_native_shortcuts_authoritative() && chord.key.starts_with("Digit"))
}

fn install_routing(window: &MainWindow, bindings: Bindings) {
    let hints = window.global::<ShortcutHints>();
    let state = bindings.clone();
    hints.on_lookup(move |action, all| state.borrow().hint_owned(action.as_str(), all));
    let state = bindings.clone();
    hints.on_lookup_help(move || state.borrow().help.clone());
    let state = bindings.clone();
    hints.on_lookup_control(move |index, searching| state.borrow().control_slot(index, searching));
    hints.set_revision(bindings.borrow().revision_text.clone());
    if window.get_keyboard_actions().row_count() == 0 {
        window.set_keyboard_actions(
            Rc::new(VecModel::from(
                shortcut_catalog()
                    .iter()
                    .map(|action| action.label.as_str().into())
                    .collect::<Vec<_>>(),
            ))
            .into(),
        );
    }
    let weak = window.as_weak();
    let state = bindings.clone();
    window.on_files_key(move |pane, event, active, composing| {
        let Some(window) = weak.upgrade() else {
            return 1;
        };
        if window.get_native_alt_gr() {
            return 0;
        }
        if shortcut_input::chord(&event)
            .is_some_and(|chord| logical_digit_is_native(&window, &chord))
        {
            return 0;
        }
        let snapshot = state.borrow().clone();
        let action = file_keys::route_with(&snapshot.map, &event, active, composing);
        file_key_bridge::dispatch(&window, pane, action)
    });
    let weak = window.as_weak();
    let state = bindings.clone();
    window.on_window_key(move |event| {
        let Some(window) = weak.upgrade() else {
            return false;
        };
        update_control_layer(&window, &event, false);
        if window.get_native_alt_gr() {
            return false;
        }
        let Some(chord) = shortcut_input::chord(&event) else {
            return false;
        };
        if logical_digit_is_native(&window, &chord) {
            return false;
        }
        let (context, pane, composing) = context(&window);
        if composing && !chord.control && !chord.alt && !chord.meta {
            return false;
        }
        let Some(file_keys::FileKey::Window(action)) = resolve(&state, &chord, context) else {
            return false;
        };
        file_key_bridge::dispatch_window(&window, pane, action);
        true
    });
    let weak = window.as_weak();
    window.on_window_key_release(move |event| {
        if let Some(window) = weak.upgrade() {
            update_control_layer(&window, &event, true);
        }
    });
    let weak = window.as_weak();
    window.on_function_action(move |id, pane| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let searching = if pane == 0 {
            window.get_left_search_active()
        } else {
            window.get_right_search_active()
        };
        let Some(action) = file_keys::semantic(id.as_str(), searching) else {
            return;
        };
        window.invoke_activate(pane);
        // Data-only commands return to the captured file pane. Actions opening
        // an editor or dialog may then establish their own focus normally.
        window.invoke_focus_files();
        let result = file_key_bridge::dispatch(&window, pane, action);
        if result >= 2 {
            window.invoke_apply_file_input_action(pane, result);
        }
    });
    let weak = window.as_weak();
    let state = bindings.clone();
    window.on_volume_key(move |event| {
        let Some(window) = weak.upgrade() else {
            return 0;
        };
        if window.get_native_alt_gr() {
            return 0;
        }
        let Some(chord) = shortcut_input::chord(&event) else {
            return 0;
        };
        if logical_digit_is_native(&window, &chord) {
            return 0;
        }
        match resolve(&state, &chord, ShortcutContext::Volume) {
            Some(file_keys::FileKey::Window(file_keys::WindowAction::OtherPanel)) => 1,
            Some(file_keys::FileKey::Window(action)) => {
                file_key_bridge::dispatch_window(&window, window.get_active_pane(), action);
                2
            }
            _ => 0,
        }
    });
    let weak = window.as_weak();
    window.on_physical_shortcut(move |key, control, alt, shift, meta| {
        let Some(window) = weak.upgrade() else {
            return false;
        };
        if window.get_native_alt_gr() {
            return false;
        }
        let chord = ShortcutChord {
            key: key.to_string(),
            control,
            alt,
            shift,
            meta,
        };
        let (context, pane, composing) = context(&window);
        if composing && !control && !alt && !meta {
            return false;
        }
        let Some(action) = resolve(&bindings, &chord, context) else {
            return false;
        };
        let result = file_key_bridge::dispatch(&window, pane, action);
        if result >= 2 {
            window.invoke_apply_file_input_action(pane, result);
        }
        result != 0
    });
}

fn update_control_layer(
    window: &MainWindow,
    event: &i_slint_core::input::KeyEvent,
    released: bool,
) {
    use slint::platform::Key;
    if window.get_native_shortcuts_authoritative() {
        return;
    }
    if event.text == slint::SharedString::from(Key::AltGr) {
        window.set_native_alt_gr(!released);
    }
    let released_control = released
        && [Key::Control, Key::ControlR]
            .into_iter()
            .any(|key| event.text == slint::SharedString::from(key));
    window.set_control_layer(
        event.modifiers.control
            && !released_control
            && !event.modifiers.alt
            && !event.modifiers.shift
            && !event.modifiers.meta
            && !window.get_native_alt_gr(),
    );
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    install_routing(window, presenter.borrow().keyboard.bindings.clone());
    let weak = window.as_weak();
    let state = presenter.clone();
    window.on_keyboard_select(move |index| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        state.borrow_mut().select_shortcut(index);
        refresh(&window, &state.borrow());
    });
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send.clone();
    window.on_keyboard_apply(move |action, text| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let keys = text
            .lines()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .collect();
        state
            .borrow_mut()
            .change_shortcuts(action.as_str(), keys, sender.as_ref());
        refresh(&window, &state.borrow());
    });
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send.clone();
    window.on_keyboard_restore(move |action| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        state
            .borrow_mut()
            .restore_shortcut(action.as_str(), sender.as_ref());
        refresh(&window, &state.borrow());
    });
    let weak = window.as_weak();
    window.on_keyboard_restore_all(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        presenter.borrow_mut().restore_shortcuts(send.as_ref());
        refresh(&window, &presenter.borrow());
    });
}

pub(crate) fn refresh_bindings(window: &MainWindow, presenter: &Presenter) {
    let state = &presenter.keyboard;
    let action = &shortcut_catalog()[state.selected];
    if window.get_keyboard_revision() == state.editor_revision_text
        && window.get_keyboard_action_id() == action.id
        && window.get_keyboard_error() == state.error
    {
        return;
    }
    window.set_keyboard_bindings(state.snapshot.editor_text(&action.id).into());
    window.set_keyboard_scope(state.snapshot.scope(&action.id).into());
    window.set_keyboard_error(state.error.as_str().into());
    window.set_keyboard_selected(state.selected as i32);
    window.set_keyboard_action_id(action.id.as_str().into());
    let hints = window.global::<ShortcutHints>();
    if hints.get_revision() != state.snapshot.revision_text {
        hints.set_revision(state.snapshot.revision_text.clone());
    }
    if window.get_keyboard_revision() != state.editor_revision_text {
        window.set_keyboard_revision(state.editor_revision_text.clone());
    }
}
