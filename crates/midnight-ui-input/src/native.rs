//! Physical number-row and modified bracket chords use the shared shortcut catalog.
use midnight_core::ShortcutChord;
use slint::winit_030::winit::{
    event::{ElementState, WindowEvent},
    keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey},
};
use std::collections::HashSet;

/// Catch Escape before a native popup or focused control can consume it.
pub fn native_escape_pressed(event: &WindowEvent) -> bool {
    matches!(event, WindowEvent::KeyboardInput { event, is_synthetic: false, .. }
        if event.state == ElementState::Pressed
            && event.logical_key == Key::Named(NamedKey::Escape))
}

struct KeyInput<'a> {
    physical: PhysicalKey,
    logical: Key<&'a str>,
    state: ElementState,
    repeat: bool,
    synthetic: bool,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Decision {
    consumed: bool,
    physical_digit: bool,
}

#[derive(Default)]
pub struct ShortcutState {
    authoritative: bool,
    modifiers: ModifiersState,
    right_alt: bool,
    physical_digit: bool,
    held: HashSet<PhysicalKey>,
    modifier_keys: HashSet<PhysicalKey>,
    // Some backends report held modifiers on focus entry without key-downs.
    untracked_modifiers: ModifiersState,
}

impl ShortcutState {
    /// Observe native modifier/key edges and dispatch physical digit/bracket shortcuts.
    /// Returns whether the native event was consumed by a shortcut.
    pub fn handle(
        &mut self,
        event: &WindowEvent,
        dispatch: impl FnMut(ShortcutChord) -> bool,
    ) -> bool {
        self.authoritative = true;
        self.observe_modifiers(event);
        if let WindowEvent::KeyboardInput {
            event,
            is_synthetic,
            ..
        } = event
        {
            let result = self.key(
                KeyInput {
                    physical: event.physical_key,
                    logical: event.logical_key.as_ref(),
                    state: event.state,
                    repeat: event.repeat,
                    synthetic: *is_synthetic,
                },
                dispatch,
            );
            self.physical_digit = result.physical_digit;
            return result.consumed;
        }
        false
    }

    /// The current native key originated from the physical number row.
    pub fn physical_digit(&self) -> bool {
        self.physical_digit
    }

    /// AltGr input belongs to the text widget, never application shortcuts.
    pub fn alt_gr(&self) -> bool {
        self.right_alt
    }

    /// Resolve logical input; software-window hosts use this without a native hook.
    pub fn logical_chord(&self, event: &i_slint_core::input::KeyEvent) -> Option<ShortcutChord> {
        if self.right_alt {
            return None;
        }
        let chord = crate::chord(event)?;
        if self.physical_digit || (self.authoritative && chord.key.starts_with("Digit")) {
            return None;
        }
        Some(chord)
    }
    fn observe_modifiers(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                self.modifier_keys
                    .retain(|key| modifier(*key).is_some_and(|flag| self.modifiers.contains(flag)));
                self.untracked_modifiers &= self.modifiers;
                for flag in [
                    ModifiersState::CONTROL,
                    ModifiersState::ALT,
                    ModifiersState::SHIFT,
                    ModifiersState::SUPER,
                ] {
                    if self.modifiers.contains(flag)
                        && !self
                            .modifier_keys
                            .iter()
                            .any(|key| modifier(*key) == Some(flag))
                    {
                        self.untracked_modifiers.insert(flag);
                    }
                }
            }
            WindowEvent::Focused(false) | WindowEvent::Destroyed => *self = Self::default(),
            _ => {}
        }
    }

    /// Current function-key modifiers: Control=1, Alt=2, Shift=4, Meta=8.
    /// AltGr belongs to text input and always returns zero.
    pub fn function_modifiers(&self) -> u8 {
        if self.right_alt {
            return 0;
        }
        u8::from(self.modifiers.control_key())
            | (u8::from(self.modifiers.alt_key()) << 1)
            | (u8::from(self.modifiers.shift_key()) << 2)
            | (u8::from(self.modifiers.super_key()) << 3)
    }

    pub fn control_layer(&self) -> bool {
        self.function_modifiers() == 1
    }

    fn key(
        &mut self,
        input: KeyInput<'_>,
        mut dispatch: impl FnMut(ShortcutChord) -> bool,
    ) -> Decision {
        let digit = digit(input.physical);
        let mut decision = Decision {
            physical_digit: digit.is_some(),
            consumed: false,
        };
        // Winit normally follows key edges with ModifiersChanged. Reflect the
        // edge immediately so a mouse release cannot use stale function actions.
        // Keep both physical sides: releasing one does not release the other.
        if let Some(flag) = modifier(input.physical) {
            if input.state == ElementState::Pressed {
                self.modifier_keys.insert(input.physical);
                // Windows reconstructs keys held across focus with synthetic
                // presses. Those explain the aggregate snapshot's prior hold;
                // a real new press on Wayland is an additional held key.
                if input.synthetic {
                    self.untracked_modifiers.remove(flag);
                }
            } else {
                self.modifier_keys.remove(&input.physical);
                // Aggregate-only focus snapshots cannot distinguish one held
                // side from both. Only the next authoritative snapshot can
                // release an untracked hold; known keys still update now.
            }
            self.modifiers.set(
                flag,
                self.untracked_modifiers.contains(flag)
                    || self
                        .modifier_keys
                        .iter()
                        .any(|key| modifier(*key) == Some(flag)),
            );
        }
        if input.physical == PhysicalKey::Code(KeyCode::AltRight)
            || matches!(input.logical, Key::Named(NamedKey::AltGraph))
        {
            self.right_alt = input.state == ElementState::Pressed;
            return decision;
        }
        if input.state == ElementState::Released {
            decision.consumed = self.held.remove(&input.physical);
            return decision;
        }
        if self.held.contains(&input.physical) {
            decision.consumed = true;
            return decision;
        }
        if input.repeat || input.synthetic || self.right_alt {
            return decision;
        }
        let key = match (digit, input.physical) {
            (Some(digit), _) => format!("Digit{digit}"),
            // These are the configured bracket key positions even when Shift or
            // a different keyboard layout changes their text. Plain/AltGr input
            // stays with the editor; no action is hardcoded into this adapter.
            (None, PhysicalKey::Code(code))
                if self.modifiers.intersects(
                    ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SUPER,
                ) =>
            {
                match code {
                    KeyCode::BracketLeft => "BracketLeft".into(),
                    KeyCode::BracketRight => "BracketRight".into(),
                    _ => return decision,
                }
            }
            _ => return decision,
        };
        let chord = ShortcutChord {
            key,
            control: self.modifiers.control_key(),
            alt: self.modifiers.alt_key(),
            shift: self.modifiers.shift_key(),
            meta: self.modifiers.super_key(),
        };
        if dispatch(chord) {
            self.held.insert(input.physical);
            decision.consumed = true;
        }
        decision
    }
}

fn modifier(key: PhysicalKey) -> Option<ModifiersState> {
    match key {
        PhysicalKey::Code(KeyCode::ControlLeft | KeyCode::ControlRight) => {
            Some(ModifiersState::CONTROL)
        }
        PhysicalKey::Code(KeyCode::AltLeft | KeyCode::AltRight) => Some(ModifiersState::ALT),
        PhysicalKey::Code(KeyCode::ShiftLeft | KeyCode::ShiftRight) => Some(ModifiersState::SHIFT),
        PhysicalKey::Code(KeyCode::SuperLeft | KeyCode::SuperRight) => Some(ModifiersState::SUPER),
        _ => None,
    }
}

fn digit(key: PhysicalKey) -> Option<u8> {
    Some(match key {
        PhysicalKey::Code(KeyCode::Digit0) => 0,
        PhysicalKey::Code(KeyCode::Digit1) => 1,
        PhysicalKey::Code(KeyCode::Digit2) => 2,
        PhysicalKey::Code(KeyCode::Digit3) => 3,
        PhysicalKey::Code(KeyCode::Digit4) => 4,
        PhysicalKey::Code(KeyCode::Digit5) => 5,
        PhysicalKey::Code(KeyCode::Digit6) => 6,
        PhysicalKey::Code(KeyCode::Digit7) => 7,
        PhysicalKey::Code(KeyCode::Digit8) => 8,
        PhysicalKey::Code(KeyCode::Digit9) => 9,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{ShortcutBinding, ShortcutContext, ShortcutMap};

    fn input(code: KeyCode, text: &'static str) -> KeyInput<'static> {
        KeyInput {
            physical: PhysicalKey::Code(code),
            logical: Key::Character(text),
            state: ElementState::Pressed,
            repeat: false,
            synthetic: false,
        }
    }

    #[test]
    fn physical_brackets_insert_each_panel_path_on_us_and_czech_layouts() {
        let map = ShortcutMap::new(&[]).unwrap();
        for (code, text, action) in [
            (KeyCode::BracketLeft, "[", "command.insert-left-path"),
            (KeyCode::BracketLeft, "{", "command.insert-left-path"),
            (KeyCode::BracketLeft, "ú", "command.insert-left-path"),
            (KeyCode::BracketRight, "]", "command.insert-right-path"),
            (KeyCode::BracketRight, "}", "command.insert-right-path"),
            (KeyCode::BracketRight, ")", "command.insert-right-path"),
        ] {
            for context in [ShortcutContext::Files, ShortcutContext::Command] {
                for modifiers in [
                    ModifiersState::CONTROL,
                    ModifiersState::CONTROL | ModifiersState::SHIFT,
                ] {
                    let mut state = ShortcutState {
                        modifiers,
                        ..Default::default()
                    };
                    let mut resolved = None;
                    let result = state.key(input(code, text), |chord| {
                        resolved = map
                            .resolve(&chord, context)
                            .map(|action| action.id.as_str());
                        resolved.is_some()
                    });
                    assert_eq!(
                        resolved,
                        Some(action),
                        "{code:?}, {text}, {context:?}, {modifiers:?}"
                    );
                    assert!(result.consumed);
                }
            }
        }
    }

    #[test]
    fn physical_brackets_honor_overrides_and_leave_plain_text_and_altgr_to_the_editor() {
        let map = ShortcutMap::new(&[
            ShortcutBinding {
                action: "command.insert-left-path".into(),
                keys: vec![],
            },
            ShortcutBinding {
                action: "command.insert-right-path".into(),
                keys: vec!["Ctrl+BracketLeft".into()],
            },
        ])
        .unwrap();
        let mut state = ShortcutState {
            modifiers: ModifiersState::CONTROL,
            ..Default::default()
        };
        let mut resolved = None;
        assert!(
            state
                .key(input(KeyCode::BracketLeft, "ú"), |chord| {
                    resolved = map
                        .resolve(&chord, ShortcutContext::Command)
                        .map(|action| action.id.as_str());
                    resolved.is_some()
                })
                .consumed
        );
        assert_eq!(resolved, Some("command.insert-right-path"));
        let mut release = input(KeyCode::BracketLeft, "ú");
        release.state = ElementState::Released;
        assert!(
            state
                .key(release, |_| panic!("release must not insert again"))
                .consumed
        );
        assert!(
            !state
                .key(input(KeyCode::BracketRight, ")"), |chord| map
                    .resolve(&chord, ShortcutContext::Command)
                    .is_some())
                .consumed
        );
        for modifiers in [ModifiersState::empty(), ModifiersState::SHIFT] {
            let mut state = ShortcutState {
                modifiers,
                ..Default::default()
            };
            assert!(
                !state
                    .key(input(KeyCode::BracketLeft, "ú"), |_| panic!(
                        "typing is not a shortcut"
                    ))
                    .consumed
            );
        }
        let mut state = ShortcutState {
            modifiers: ModifiersState::CONTROL | ModifiersState::ALT,
            right_alt: true,
            ..Default::default()
        };
        assert!(
            !state
                .key(input(KeyCode::BracketRight, "]"), |_| panic!(
                    "AltGr belongs to the editor"
                ))
                .consumed
        );
    }

    #[test]
    fn viewer_bindings_resolve_physical_digits_on_us_and_czech_layouts() {
        let map = ShortcutMap::new(&[
            // Reassign the image viewer's default chord explicitly, as settings require.
            ShortcutBinding {
                action: "viewer.actual-pixels".into(),
                keys: Vec::new(),
            },
            ShortcutBinding {
                action: "viewer.next-file".into(),
                keys: vec!["Ctrl+Digit1".into()],
            },
            ShortcutBinding {
                action: "viewer.find-next".into(),
                keys: vec!["Ctrl+Alt+Digit1".into()],
            },
        ])
        .unwrap();
        for (context, modifiers, expected) in [
            (
                ShortcutContext::Viewer,
                ModifiersState::CONTROL,
                Some("viewer.next-file"),
            ),
            (ShortcutContext::ViewerSearch, ModifiersState::CONTROL, None),
            (
                ShortcutContext::ViewerSearch,
                ModifiersState::CONTROL | ModifiersState::ALT,
                Some("viewer.find-next"),
            ),
        ] {
            for text in ["1", "+", "!"] {
                let mut state = ShortcutState {
                    modifiers,
                    ..Default::default()
                };
                let mut resolved = None;
                let result = state.key(input(KeyCode::Digit1, text), |chord| {
                    resolved = map
                        .resolve(&chord, context)
                        .map(|action| action.id.as_str());
                    resolved.is_some()
                });
                assert_eq!(resolved, expected, "logical text {text} in {context:?}");
                assert_eq!(result.consumed, expected.is_some());
            }
        }
    }

    #[test]
    fn native_digits_and_altgr_do_not_dispatch_a_second_logical_shortcut() {
        let mut event = i_slint_core::input::KeyEvent::default();
        event.text = "1".into();
        event.modifiers.control = true;
        let mut state = ShortcutState::default();
        assert_eq!(
            state.logical_chord(&event).unwrap().display(),
            "Ctrl+Digit1"
        );
        state.handle(&WindowEvent::Focused(true), |_| false);
        assert!(
            state.logical_chord(&event).is_none(),
            "native digit already resolved physically"
        );
        state.physical_digit = true;
        event.text = "+".into();
        assert!(
            state.logical_chord(&event).is_none(),
            "number-row text is not another binding"
        );
        state.physical_digit = false;
        state.key(input(KeyCode::AltRight, ""), |_| false);
        event.text = "@".into();
        event.modifiers.alt = true;
        assert!(
            state.logical_chord(&event).is_none(),
            "AltGr must stay in the native text editor"
        );
    }

    #[test]
    fn bare_physical_digits_navigate_only_in_file_context() {
        let map = ShortcutMap::new(&[]).unwrap();
        for (context, consumed) in [
            (ShortcutContext::Files, true),
            (ShortcutContext::Search, false),
            (ShortcutContext::Command, false),
            (ShortcutContext::Other, false),
        ] {
            for text in ["1", "+"] {
                let mut state = ShortcutState::default();
                let decision = state.key(input(KeyCode::Digit1, text), |chord| {
                    map.resolve(&chord, context).is_some()
                });
                assert_eq!(decision.consumed, consumed);
            }
        }
    }

    #[test]
    fn function_modifiers_cover_all_native_combinations() {
        for (expected, modifiers) in [
            (0, ModifiersState::empty()),
            (1, ModifiersState::CONTROL),
            (2, ModifiersState::ALT),
            (3, ModifiersState::CONTROL | ModifiersState::ALT),
            (4, ModifiersState::SHIFT),
            (5, ModifiersState::CONTROL | ModifiersState::SHIFT),
            (6, ModifiersState::ALT | ModifiersState::SHIFT),
            (
                7,
                ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT,
            ),
            (8, ModifiersState::SUPER),
            (9, ModifiersState::CONTROL | ModifiersState::SUPER),
            (10, ModifiersState::ALT | ModifiersState::SUPER),
            (
                11,
                ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SUPER,
            ),
            (12, ModifiersState::SHIFT | ModifiersState::SUPER),
            (
                13,
                ModifiersState::CONTROL | ModifiersState::SHIFT | ModifiersState::SUPER,
            ),
            (
                14,
                ModifiersState::ALT | ModifiersState::SHIFT | ModifiersState::SUPER,
            ),
            (15, ModifiersState::all()),
        ] {
            let mut state = ShortcutState::default();
            assert!(!state.handle(&WindowEvent::ModifiersChanged(modifiers.into()), |_| false));
            assert_eq!(state.function_modifiers(), expected, "{modifiers:?}");
            assert_eq!(state.control_layer(), expected == 1);
        }
    }

    #[test]
    fn function_modifiers_follow_key_edges_and_preserve_other_held_sides() {
        for (expected, left, right) in [
            (1, KeyCode::ControlLeft, KeyCode::ControlRight),
            (4, KeyCode::ShiftLeft, KeyCode::ShiftRight),
            (8, KeyCode::SuperLeft, KeyCode::SuperRight),
        ] {
            let mut state = ShortcutState::default();
            state.key(input(left, ""), |_| panic!("modifier is not a shortcut"));
            assert_eq!(state.function_modifiers(), expected);
            state.key(input(right, ""), |_| false);
            let mut release = input(left, "");
            release.state = ElementState::Released;
            state.key(release, |_| false);
            assert_eq!(
                state.function_modifiers(),
                expected,
                "other side remains held"
            );
            let mut release = input(right, "");
            release.state = ElementState::Released;
            state.key(release, |_| false);
            assert_eq!(
                state.function_modifiers(),
                0,
                "release precedes ModifiersChanged"
            );
        }
        let mut state = ShortcutState::default();
        for (code, expected) in [
            (KeyCode::ControlLeft, 1),
            (KeyCode::AltLeft, 3),
            (KeyCode::ShiftLeft, 7),
            (KeyCode::SuperLeft, 15),
        ] {
            state.key(input(code, ""), |_| false);
            assert_eq!(state.function_modifiers(), expected);
        }
        for (code, expected) in [
            (KeyCode::AltLeft, 13),
            (KeyCode::ControlLeft, 12),
            (KeyCode::SuperLeft, 4),
            (KeyCode::ShiftLeft, 0),
        ] {
            let mut release = input(code, "");
            release.state = ElementState::Released;
            state.key(release, |_| false);
            assert_eq!(state.function_modifiers(), expected);
        }
    }

    #[test]
    fn function_modifiers_reconcile_snapshots_and_clear_after_focus_loss() {
        for end in [WindowEvent::Focused(false), WindowEvent::Destroyed] {
            let mut state = ShortcutState::default();
            for code in [
                KeyCode::ControlLeft,
                KeyCode::AltLeft,
                KeyCode::ShiftLeft,
                KeyCode::SuperLeft,
            ] {
                state.key(input(code, ""), |_| false);
            }
            state.handle(&end, |_| false);
            assert_eq!(state.function_modifiers(), 0);
            state.handle(&WindowEvent::Focused(true), |_| false);
            assert_eq!(
                state.function_modifiers(),
                0,
                "focus must not revive modifiers"
            );
            state.key(input(KeyCode::ShiftRight, ""), |_| false);
            let mut release = input(KeyCode::ShiftRight, "");
            release.state = ElementState::Released;
            state.key(release, |_| false);
            assert_eq!(
                state.function_modifiers(),
                0,
                "old left Shift was forgotten"
            );
        }
        let mut state = ShortcutState::default();
        state.key(input(KeyCode::ShiftLeft, ""), |_| false);
        state.handle(
            &WindowEvent::ModifiersChanged(ModifiersState::empty().into()),
            |_| false,
        );
        state.key(input(KeyCode::ShiftRight, ""), |_| false);
        let mut release = input(KeyCode::ShiftRight, "");
        release.state = ElementState::Released;
        state.key(release, |_| false);
        assert_eq!(
            state.function_modifiers(),
            0,
            "snapshot removed the stale left Shift"
        );
    }

    #[test]
    fn function_modifiers_snapshot_only_holds_survive_opposite_side_key_edges() {
        for (flag, expected, left, right) in [
            (
                ModifiersState::CONTROL,
                1,
                KeyCode::ControlLeft,
                KeyCode::ControlRight,
            ),
            (
                ModifiersState::SHIFT,
                4,
                KeyCode::ShiftLeft,
                KeyCode::ShiftRight,
            ),
            (
                ModifiersState::SUPER,
                8,
                KeyCode::SuperLeft,
                KeyCode::SuperRight,
            ),
        ] {
            for (original, additional) in [(left, right), (right, left)] {
                let mut state = ShortcutState::default();
                // Wayland reports the aggregate held modifiers on focus entry,
                // without synthesizing key-downs for those already held keys.
                state.handle(&WindowEvent::Focused(true), |_| false);
                state.handle(&WindowEvent::ModifiersChanged(flag.into()), |_| false);
                state.key(input(additional, ""), |_| false);
                state.handle(&WindowEvent::ModifiersChanged(flag.into()), |_| false);
                let mut release = input(additional, "");
                release.state = ElementState::Released;
                state.key(release, |_| false);
                assert_eq!(
                    state.function_modifiers(),
                    expected,
                    "{original:?} remains held"
                );
                let mut release = input(original, "");
                release.state = ElementState::Released;
                state.key(release, |_| false);
                assert_eq!(
                    state.function_modifiers(),
                    expected,
                    "unknown side awaits snapshot"
                );
                state.handle(
                    &WindowEvent::ModifiersChanged(ModifiersState::empty().into()),
                    |_| false,
                );
                assert_eq!(state.function_modifiers(), 0, "authoritative last release");
            }
        }
    }

    #[test]
    fn function_modifiers_snapshot_only_last_release_waits_for_authoritative_snapshot() {
        for (flag, key) in [
            (ModifiersState::CONTROL, KeyCode::ControlRight),
            (ModifiersState::ALT, KeyCode::AltLeft),
            (ModifiersState::SHIFT, KeyCode::ShiftLeft),
            (ModifiersState::SUPER, KeyCode::SuperRight),
        ] {
            let mut state = ShortcutState::default();
            state.handle(&WindowEvent::ModifiersChanged(flag.into()), |_| false);
            let mut release = input(key, "");
            release.state = ElementState::Released;
            state.key(release, |_| false);
            assert!(
                state.modifiers.contains(flag),
                "unknown count cannot be inferred from one key-up"
            );
            state.handle(
                &WindowEvent::ModifiersChanged(ModifiersState::empty().into()),
                |_| false,
            );
            assert_eq!(
                state.function_modifiers(),
                0,
                "authoritative last release cannot stick"
            );
            state.handle(&WindowEvent::ModifiersChanged(flag.into()), |_| false);
            state.handle(
                &WindowEvent::ModifiersChanged(ModifiersState::empty().into()),
                |_| false,
            );
            state.key(input(key, ""), |_| false);
            let mut release = input(key, "");
            release.state = ElementState::Released;
            state.key(release, |_| false);
            assert_eq!(
                state.function_modifiers(),
                0,
                "snapshot cleared the previous unknown hold"
            );
        }
    }

    #[test]
    fn function_modifiers_two_untracked_sides_keep_the_unchanged_aggregate_after_one_release() {
        for (flag, expected, left, right) in [
            (
                ModifiersState::CONTROL,
                1,
                KeyCode::ControlLeft,
                KeyCode::ControlRight,
            ),
            (
                ModifiersState::SHIFT,
                4,
                KeyCode::ShiftLeft,
                KeyCode::ShiftRight,
            ),
            (
                ModifiersState::SUPER,
                8,
                KeyCode::SuperLeft,
                KeyCode::SuperRight,
            ),
        ] {
            let mut state = ShortcutState::default();
            // Both keys were held before focus, but Wayland provides only one bit.
            state.handle(&WindowEvent::ModifiersChanged(flag.into()), |_| false);
            let mut release = input(left, "");
            release.state = ElementState::Released;
            state.key(release, |_| false);
            assert_eq!(state.function_modifiers(), expected);
            state.handle(&WindowEvent::ModifiersChanged(flag.into()), |_| false);
            assert_eq!(
                state.function_modifiers(),
                expected,
                "other side remains held"
            );
            let mut release = input(right, "");
            release.state = ElementState::Released;
            state.key(release, |_| false);
            assert_eq!(
                state.function_modifiers(),
                expected,
                "last unknown release awaits snapshot"
            );
            state.handle(
                &WindowEvent::ModifiersChanged(ModifiersState::empty().into()),
                |_| false,
            );
            assert_eq!(state.function_modifiers(), 0);
        }
    }

    #[test]
    fn function_modifiers_synthetic_focus_keys_resolve_snapshot_only_holds() {
        for (flag, expected, left, right) in [
            (
                ModifiersState::CONTROL,
                1,
                KeyCode::ControlLeft,
                KeyCode::ControlRight,
            ),
            (
                ModifiersState::SHIFT,
                4,
                KeyCode::ShiftLeft,
                KeyCode::ShiftRight,
            ),
            (
                ModifiersState::SUPER,
                8,
                KeyCode::SuperLeft,
                KeyCode::SuperRight,
            ),
        ] {
            let mut state = ShortcutState::default();
            state.handle(&WindowEvent::ModifiersChanged(flag.into()), |_| false);
            for code in [left, right] {
                let mut press = input(code, "");
                press.synthetic = true;
                state.key(press, |_| panic!("synthetic focus keys must not dispatch"));
            }
            state.handle(&WindowEvent::Focused(true), |_| false);
            state.handle(&WindowEvent::ModifiersChanged(flag.into()), |_| false);
            let mut release = input(left, "");
            release.state = ElementState::Released;
            state.key(release, |_| false);
            assert_eq!(state.function_modifiers(), expected);
            let mut release = input(right, "");
            release.state = ElementState::Released;
            state.key(release, |_| false);
            assert_eq!(
                state.function_modifiers(),
                0,
                "last known release precedes snapshot"
            );
        }
    }

    #[test]
    fn function_modifiers_exclude_altgr_until_its_release() {
        let mut state = ShortcutState::default();
        state.key(input(KeyCode::ControlLeft, ""), |_| false);
        state.key(input(KeyCode::AltRight, ""), |_| false);
        state.handle(
            &WindowEvent::ModifiersChanged(ModifiersState::all().into()),
            |_| false,
        );
        assert_eq!(state.function_modifiers(), 0);
        assert!(!state.control_layer());
        let mut release = input(KeyCode::AltRight, "");
        release.state = ElementState::Released;
        state.key(release, |_| false);
        assert_eq!(
            state.function_modifiers(),
            13,
            "released AltGr removes Alt immediately"
        );
        state.handle(&WindowEvent::Focused(false), |_| false);
        assert_eq!(state.function_modifiers(), 0);
        assert!(!state.alt_gr());
    }

    #[test]
    fn control_footer_tracks_both_control_keys_and_clears_on_native_focus_loss() {
        let mut state = ShortcutState::default();
        state.key(input(KeyCode::ControlLeft, ""), |_| false);
        state.key(input(KeyCode::ControlRight, ""), |_| false);
        assert!(state.control_layer());
        let mut release = input(KeyCode::ControlLeft, "");
        release.state = ElementState::Released;
        state.key(release, |_| false);
        assert!(state.control_layer(), "the other Ctrl is still held");
        state.key(input(KeyCode::AltRight, ""), |_| false);
        assert!(
            !state.control_layer(),
            "AltGr never activates the Ctrl footer"
        );
        state.observe_modifiers(&WindowEvent::Focused(false));
        assert!(!state.control_layer());
        assert!(!state.right_alt);
        assert!(state.modifier_keys.is_empty());
        assert!(state.held.is_empty());
        state.observe_modifiers(&WindowEvent::Focused(true));
        assert!(
            !state.control_layer(),
            "focus restoration does not revive a stale key"
        );
        state.key(input(KeyCode::ControlRight, ""), |_| false);
        assert!(state.control_layer());
        let mut release = input(KeyCode::ControlRight, "");
        release.state = ElementState::Released;
        state.key(release, |_| false);
        assert!(
            !state.control_layer(),
            "key-up clears before ModifiersChanged arrives"
        );
    }

    #[test]
    fn physical_digits_use_configured_actions_instead_of_logical_punctuation() {
        let map = ShortcutMap::new(&[
            ShortcutBinding {
                action: "bookmark.open.1".into(),
                keys: vec![],
            },
            ShortcutBinding {
                action: "files.copy".into(),
                keys: vec!["Ctrl+Digit1".into()],
            },
        ])
        .unwrap();
        for text in ["1", "+", "!"] {
            let mut state = ShortcutState {
                modifiers: ModifiersState::CONTROL,
                ..Default::default()
            };
            let mut actions = Vec::new();
            let result = state.key(input(KeyCode::Digit1, text), |chord| {
                let action = map.resolve(&chord, ShortcutContext::Files).unwrap();
                actions.push(action.id.clone());
                true
            });
            assert_eq!(actions, ["files.copy"]);
            assert!(result.consumed && result.physical_digit);
        }
    }

    #[test]
    fn unbound_digits_and_search_typing_still_mark_the_logical_event_as_physical() {
        let map = ShortcutMap::new(&[ShortcutBinding {
            action: "bookmark.open.1".into(),
            keys: vec![],
        }])
        .unwrap();
        for context in [
            ShortcutContext::Files,
            ShortcutContext::Search,
            ShortcutContext::Command,
        ] {
            let mut state = ShortcutState {
                modifiers: ModifiersState::CONTROL,
                ..Default::default()
            };
            let result = state.key(input(KeyCode::Digit1, "+"), |chord| {
                map.resolve(&chord, context).is_some()
            });
            assert_eq!(
                result,
                Decision {
                    consumed: false,
                    physical_digit: true
                }
            );
        }
        let map = ShortcutMap::new(&[]).unwrap();
        let mut state = ShortcutState {
            modifiers: ModifiersState::SHIFT,
            ..Default::default()
        };
        assert!(
            !state
                .key(input(KeyCode::Digit1, "!"), |chord| map
                    .resolve(&chord, ShortcutContext::Search)
                    .is_some())
                .consumed
        );
    }

    #[test]
    fn alt_gr_never_dispatches_and_left_alt_can_address_the_other_panel() {
        let map = ShortcutMap::new(&[]).unwrap();
        let mut state = ShortcutState::default();
        // The left keys stay physically held when the right Alt is released.
        for code in [KeyCode::ControlLeft, KeyCode::AltLeft] {
            state.key(input(code, ""), |_| panic!("modifier cannot dispatch"));
        }
        state.key(input(KeyCode::AltRight, ""), |_| {
            panic!("modifier cannot dispatch")
        });
        assert!(state.right_alt);
        assert!(
            !state
                .key(input(KeyCode::Digit1, "+"), |_| panic!(
                    "AltGr cannot dispatch"
                ))
                .consumed
        );
        let mut release = input(KeyCode::AltRight, "");
        release.state = ElementState::Released;
        state.key(release, |_| false);
        assert!(!state.right_alt);
        assert!(
            state
                .key(input(KeyCode::Digit1, "1"), |chord| {
                    assert_eq!(
                        map.resolve(&chord, ShortcutContext::Files).unwrap().id,
                        "bookmark.other.1"
                    );
                    true
                })
                .consumed
        );
    }

    #[test]
    fn repeats_synthetic_input_and_releases_cannot_dispatch_twice() {
        let mut state = ShortcutState::default();
        assert!(state.key(input(KeyCode::Digit1, "1"), |_| true).consumed);
        let mut repeat = input(KeyCode::Digit1, "1");
        repeat.repeat = true;
        assert!(
            state
                .key(repeat, |_| panic!("held key must not dispatch"))
                .consumed
        );
        let mut release = input(KeyCode::Digit1, "1");
        release.state = ElementState::Released;
        assert!(
            state
                .key(release, |_| panic!("release must not dispatch"))
                .consumed
        );
        let mut synthetic = input(KeyCode::Digit1, "1");
        synthetic.synthetic = true;
        assert!(
            !state
                .key(synthetic, |_| panic!("synthetic key must not dispatch"))
                .consumed
        );
        assert!(state.held.is_empty());
    }

    #[test]
    fn default_tab_numbers_and_custom_meta_chords_resolve_through_the_catalog() {
        for (mods, binding, expected) in [
            (
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                None,
                "tab.1",
            ),
            (ModifiersState::SUPER, Some("Meta+Digit1"), "files.copy"),
        ] {
            let overrides = binding
                .map(|keys| {
                    vec![ShortcutBinding {
                        action: "files.copy".into(),
                        keys: vec![keys.into()],
                    }]
                })
                .unwrap_or_default();
            let map = ShortcutMap::new(&overrides).unwrap();
            let mut state = ShortcutState {
                modifiers: mods,
                ..Default::default()
            };
            assert!(
                state
                    .key(input(KeyCode::Digit1, "!"), |chord| {
                        assert_eq!(
                            map.resolve(&chord, ShortcutContext::Files).unwrap().id,
                            expected
                        );
                        true
                    })
                    .consumed
            );
        }
    }
}
