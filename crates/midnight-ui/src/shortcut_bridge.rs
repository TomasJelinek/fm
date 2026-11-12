//! Shared routing and cached presentation for standalone and presenter-backed UI.
use crate::{
    MainWindow, Presenter, ShortcutHints, file_key_bridge, file_keys,
    presenter::keyboard::{KeyboardBindings, KeyboardState},
    refresh, shortcut_input,
};
use midnight_core::{Command, ShortcutChord, ShortcutContext};
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
    let action = snapshot.map.resolve(chord, context).or_else(|| {
        // Popups retain their own navigation/global bindings. Only saved-path
        // actions may fall back to the user's configured file-list bindings.
        (context == ShortcutContext::Volume)
            .then(|| snapshot.map.resolve(chord, ShortcutContext::Files))
            .flatten()
            .filter(|action| {
                action.id.starts_with("bookmark.open.") || action.id.starts_with("bookmark.other.")
            })
    })?;
    file_keys::semantic(&action.id, context == ShortcutContext::Search)
}

fn dispatch_window(
    window: &MainWindow,
    pane: i32,
    context: ShortcutContext,
    action: file_keys::WindowAction,
) {
    if context == ShortcutContext::Volume
        && let file_keys::WindowAction::BookmarkOpen(slot, other) = action
    {
        let choices = if window.get_hot_path_popup_pane() == pane {
            window.get_hot_path_choices()
        } else {
            window.get_navigation_choices()
        };
        let label = (slot % 10).to_string();
        let index = choices.iter().position(|row| {
            row.kind == crate::LocationRole::HotPath && row.compact_label == label.as_str()
        });
        if let Some(index) = index {
            let identity = window.invoke_take_location_choice(pane, index as i32);
            if !identity.is_empty() {
                window.invoke_bookmark_open(if other { 1 - pane } else { pane }, identity);
            }
        }
        return;
    }
    file_key_bridge::dispatch_window(window, pane, action);
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
    hints.on_lookup_function(move |index, modifiers, searching| {
        state.borrow().function_slot(index, modifiers, searching)
    });
    hints.set_revision(bindings.borrow().revision_text.clone());
    if window.get_keyboard_actions().row_count() == 0 {
        window.set_keyboard_actions(
            Rc::new(VecModel::from(
                bindings
                    .borrow()
                    .map
                    .definitions()
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
        if crate::shortcut_editor::capture(&window, &event) {
            return true;
        }
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
        dispatch_window(&window, pane, context, action);
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
            Some(file_keys::FileKey::Window(action)) => {
                dispatch_window(
                    &window,
                    window.get_volume_popup_pane(),
                    ShortcutContext::Volume,
                    action,
                );
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
        if crate::shortcut_editor::capture_physical(&window, &chord) {
            return true;
        }
        let (context, pane, composing) = context(&window);
        if composing && !control && !alt && !meta {
            return false;
        }
        let Some(action) = resolve(&bindings, &chord, context) else {
            return false;
        };
        if let file_keys::FileKey::Window(action) = action {
            dispatch_window(&window, pane, context, action);
            return true;
        }
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
    let alt_gr_edge = event.text == slint::SharedString::from(Key::AltGr);
    if alt_gr_edge {
        window.set_native_alt_gr(!released);
    }
    let mut mask = i32::from(event.modifiers.control)
        | (i32::from(event.modifiers.alt) << 1)
        | (i32::from(event.modifiers.shift) << 2)
        | (i32::from(event.modifiers.meta) << 3);
    // Software hosts can deliver a modifier edge before updating its flags.
    // Other modifiers remain exactly as reported by the event snapshot.
    for (key, bit) in [
        (Key::Control, 1),
        (Key::ControlR, 1),
        (Key::Alt, 2),
        (Key::Shift, 4),
        (Key::ShiftR, 4),
        (Key::Meta, 8),
        (Key::MetaR, 8),
    ] {
        if event.text == slint::SharedString::from(key) {
            if released {
                mask &= !bit;
            } else {
                mask |= bit;
            }
            break;
        }
    }
    // AltGr key-up may still contain its synthesized Ctrl+Alt flags.
    if alt_gr_edge || window.get_native_alt_gr() {
        mask = 0;
    }
    window.set_function_modifiers(mask);
    window.set_control_layer(mask == 1);
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    install_routing(window, presenter.borrow().keyboard.bindings.clone());
    crate::shortcut_editor::install(window, presenter.clone(), send.clone());
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
    crate::shortcut_editor::refresh(window, presenter);
    let state = &presenter.keyboard;
    let action = &state.snapshot.map.definitions()[state.selected];
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
        window.set_keyboard_actions(
            Rc::new(VecModel::from(
                state
                    .snapshot
                    .map
                    .definitions()
                    .iter()
                    .map(|definition| definition.label.as_str().into())
                    .collect::<Vec<_>>(),
            ))
            .into(),
        );
        hints.set_revision(state.snapshot.revision_text.clone());
    }
    if window.get_keyboard_revision() != state.editor_revision_text {
        window.set_keyboard_revision(state.editor_revision_text.clone());
    }
}

#[cfg(test)]
mod function_modifier_tests {
    use super::*;
    use i_slint_core::input::KeyEvent;
    use slint::platform::{
        Key, Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    };

    fn ui(test: impl FnOnce(MainWindow) + Send + 'static) {
        std::thread::spawn(move || {
            struct TestPlatform;
            impl Platform for TestPlatform {
                fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
                    Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
                }
            }
            slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
            test(MainWindow::new().unwrap());
        })
        .join()
        .unwrap();
    }

    fn event(key: Key, mask: i32) -> KeyEvent {
        let mut event = KeyEvent::default();
        event.text = key.into();
        event.modifiers.control = mask & 1 != 0;
        event.modifiers.alt = mask & 2 != 0;
        event.modifiers.shift = mask & 4 != 0;
        event.modifiers.meta = mask & 8 != 0;
        event
    }

    #[test]
    fn function_modifiers_logical_events_cover_all_combinations_and_modifier_edges() {
        ui(|window| {
            for mask in 0..16 {
                update_control_layer(&window, &event(Key::F1, mask), false);
                assert_eq!(window.get_function_modifiers(), mask);
                assert_eq!(window.get_control_layer(), mask == 1);
            }
            for (key, bit) in [
                (Key::Control, 1),
                (Key::ControlR, 1),
                (Key::Alt, 2),
                (Key::Shift, 4),
                (Key::ShiftR, 4),
                (Key::Meta, 8),
                (Key::MetaR, 8),
            ] {
                update_control_layer(&window, &event(key, 0), false);
                assert_eq!(window.get_function_modifiers(), bit, "press precedes flags");
                update_control_layer(&window, &event(key, 15), true);
                assert_eq!(
                    window.get_function_modifiers(),
                    15 ^ bit,
                    "release precedes flags"
                );
                assert!(!window.get_control_layer());
            }
        });
    }

    #[test]
    fn function_modifiers_logical_altgr_never_exposes_ctrl_alt_or_overrides_native_state() {
        ui(|window| {
            update_control_layer(&window, &event(Key::AltGr, 3), false);
            assert_eq!(window.get_function_modifiers(), 0);
            assert!(window.get_native_alt_gr());
            update_control_layer(&window, &event(Key::F1, 15), false);
            assert_eq!(window.get_function_modifiers(), 0);
            update_control_layer(&window, &event(Key::AltGr, 3), true);
            assert_eq!(
                window.get_function_modifiers(),
                0,
                "AltGr key-up still has old Ctrl+Alt flags"
            );
            assert!(!window.get_native_alt_gr());
            update_control_layer(&window, &event(Key::F1, 4), false);
            assert_eq!(window.get_function_modifiers(), 4);
            window.set_native_shortcuts_authoritative(true);
            window.set_function_modifiers(9);
            update_control_layer(&window, &event(Key::Shift, 4), false);
            assert_eq!(
                window.get_function_modifiers(),
                9,
                "native state owns the footer"
            );
        });
    }

    #[test]
    fn function_modifiers_native_bridge_publishes_combinations_and_focus_reset() {
        use slint::winit_030::winit::{event::WindowEvent, keyboard::ModifiersState};
        ui(|window| {
            let mut state = crate::native_shortcuts::ShortcutState::default();
            for mask in 0..16 {
                let mut modifiers = ModifiersState::empty();
                modifiers.set(ModifiersState::CONTROL, mask & 1 != 0);
                modifiers.set(ModifiersState::ALT, mask & 2 != 0);
                modifiers.set(ModifiersState::SHIFT, mask & 4 != 0);
                modifiers.set(ModifiersState::SUPER, mask & 8 != 0);
                assert!(!state.handle(&window, &WindowEvent::ModifiersChanged(modifiers.into())));
                assert_eq!(window.get_function_modifiers(), mask);
                assert_eq!(window.get_control_layer(), mask == 1);
            }
            assert!(!state.handle(&window, &WindowEvent::Focused(false)));
            assert_eq!(window.get_function_modifiers(), 0);
            assert!(!window.get_control_layer());
            assert!(!state.handle(&window, &WindowEvent::Focused(true)));
            assert_eq!(window.get_function_modifiers(), 0);
        });
    }
}
