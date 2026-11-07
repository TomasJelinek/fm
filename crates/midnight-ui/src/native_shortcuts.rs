//! Physical digit chords are resolved by the same catalog as logical UI keys.
use crate::MainWindow;
use midnight_core::ShortcutChord;
use slint::winit_030::winit::{
    event::{ElementState, WindowEvent},
    keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey},
};
use std::collections::HashSet;

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
pub(crate) struct ShortcutState {
    modifiers: ModifiersState,
    right_alt: bool,
    held: HashSet<PhysicalKey>,
    controls: HashSet<PhysicalKey>,
}

impl ShortcutState {
    pub(crate) fn handle(&mut self, window: &MainWindow, event: &WindowEvent) -> bool {
        self.observe_modifiers(event);
        match event {
            WindowEvent::Focused(false) | WindowEvent::Destroyed => {
                window.set_native_digit_event(false);
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                let result = self.key(
                    KeyInput {
                        physical: event.physical_key,
                        logical: event.logical_key.as_ref(),
                        state: event.state,
                        repeat: event.repeat,
                        synthetic: *is_synthetic,
                    },
                    |chord| {
                        window.invoke_physical_shortcut(
                            chord.key.into(),
                            chord.control,
                            chord.alt,
                            chord.shift,
                            chord.meta,
                        )
                    },
                );
                window.set_native_digit_event(result.physical_digit);
                window.set_native_alt_gr(self.right_alt);
                window.set_control_layer(self.control_layer());
                return result.consumed;
            }
            _ => {}
        }
        window.set_native_alt_gr(self.right_alt);
        window.set_control_layer(self.control_layer());
        false
    }

    fn observe_modifiers(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                if !self.modifiers.control_key() {
                    self.controls.clear();
                }
            }
            WindowEvent::Focused(false) | WindowEvent::Destroyed => *self = Self::default(),
            _ => {}
        }
    }

    fn control_layer(&self) -> bool {
        self.modifiers == ModifiersState::CONTROL && !self.right_alt
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
        // edge immediately so a mouse release cannot see a stale Ctrl layer.
        if matches!(
            input.physical,
            PhysicalKey::Code(KeyCode::ControlLeft | KeyCode::ControlRight)
        ) {
            if input.state == ElementState::Pressed {
                self.controls.insert(input.physical);
            } else {
                self.controls.remove(&input.physical);
            }
            self.modifiers
                .set(ModifiersState::CONTROL, !self.controls.is_empty());
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
        let Some(digit) = digit else {
            return decision;
        };
        let chord = ShortcutChord {
            key: format!("Digit{digit}"),
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
        assert!(state.controls.is_empty());
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
        let mut state = ShortcutState {
            modifiers: ModifiersState::CONTROL | ModifiersState::ALT,
            ..Default::default()
        };
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
