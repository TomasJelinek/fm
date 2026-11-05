//! Physical Hot Path digits keep their meaning across keyboard layouts.
use crate::MainWindow;
use slint::winit_030::winit::{
    event::{ElementState, WindowEvent},
    keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey},
};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Jump(i32, i32),
    Assign(i32, i32),
    Tab(i32, i32),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Decision {
    consume: bool,
    action: Option<Action>,
}

struct KeyInput<'a> {
    physical: PhysicalKey,
    logical: Key<&'a str>,
    state: ElementState,
    repeat: bool,
    synthetic: bool,
}

#[derive(Default)]
pub(crate) struct ShortcutState {
    modifiers: ModifiersState,
    right_alt: bool,
    held: HashSet<PhysicalKey>,
    search_active: bool,
}

impl ShortcutState {
    pub(crate) fn handle(&mut self, window: &MainWindow, event: &WindowEvent) -> bool {
        self.search_active = match window.get_focused_pane() {
            0 => window.get_left_search_active(),
            1 => window.get_right_search_active(),
            _ => false,
        };
        let decision = self.observe(event, window.get_focused_pane());
        match decision.action {
            Some(Action::Jump(pane, slot)) => window.invoke_bookmark_jump(pane, slot),
            Some(Action::Assign(pane, slot)) => window.invoke_bookmark_slot(pane, slot),
            Some(Action::Tab(pane, number)) => window.invoke_tab_number(pane, number),
            None => {}
        }
        decision.consume
    }

    fn observe(&mut self, event: &WindowEvent, pane: i32) -> Decision {
        match event {
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(false) | WindowEvent::Destroyed => *self = Self::default(),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                return self.key(
                    KeyInput {
                        physical: event.physical_key,
                        logical: event.logical_key.as_ref(),
                        state: event.state,
                        repeat: event.repeat,
                        synthetic: *is_synthetic,
                    },
                    pane,
                );
            }
            _ => {}
        }
        Decision::default()
    }

    fn key(&mut self, input: KeyInput<'_>, pane: i32) -> Decision {
        if input.physical == PhysicalKey::Code(KeyCode::AltRight)
            || matches!(input.logical, Key::Named(NamedKey::AltGraph))
        {
            self.right_alt = input.state == ElementState::Pressed;
            return Decision::default();
        }
        let files_focused = matches!(pane, 0 | 1);
        if input.state == ElementState::Released {
            return Decision {
                consume: self.held.remove(&input.physical) && files_focused,
                action: None,
            };
        }
        if !files_focused || input.synthetic {
            return Decision::default();
        }
        if self.held.contains(&input.physical) {
            return Decision {
                consume: true,
                action: None,
            };
        }
        if input.repeat || self.right_alt || self.modifiers.super_key() {
            return Decision::default();
        }
        let slot = match input.physical {
            PhysicalKey::Code(KeyCode::Digit1) => 1,
            PhysicalKey::Code(KeyCode::Digit2) => 2,
            PhysicalKey::Code(KeyCode::Digit3) => 3,
            PhysicalKey::Code(KeyCode::Digit4) => 4,
            PhysicalKey::Code(KeyCode::Digit5) => 5,
            PhysicalKey::Code(KeyCode::Digit6) => 6,
            PhysicalKey::Code(KeyCode::Digit7) => 7,
            PhysicalKey::Code(KeyCode::Digit8) => 8,
            PhysicalKey::Code(KeyCode::Digit9) => 9,
            PhysicalKey::Code(KeyCode::Digit0) => 10,
            _ => return Decision::default(),
        };
        let control = self.modifiers.control_key();
        let shift = self.modifiers.shift_key();
        let alt = self.modifiers.alt_key();
        if self.search_active && shift && !control && !alt {
            return Decision::default();
        }
        let action = match (control, shift, alt) {
            (true, true, false) if slot <= 9 => Action::Tab(pane, slot),
            (true, true, false) => Action::Assign(pane, slot),
            (true, false, _) | (false, true, _) => {
                Action::Jump(if alt { 1 - pane } else { pane }, slot)
            }
            _ => return Decision::default(),
        };
        self.held.insert(input.physical);
        Decision {
            consume: true,
            action: Some(action),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::winit_030::winit::keyboard::NamedKey;

    fn input(code: KeyCode, text: &'static str) -> KeyInput<'static> {
        KeyInput {
            physical: PhysicalKey::Code(code),
            logical: Key::Character(text),
            state: ElementState::Pressed,
            repeat: false,
            synthetic: false,
        }
    }
    fn modifiers(state: &mut ShortcutState, value: ModifiersState) {
        state.observe(&WindowEvent::ModifiersChanged(value.into()), -1);
    }
    fn accepted(action: Action) -> Decision {
        Decision {
            consume: true,
            action: Some(action),
        }
    }

    #[test]
    fn physical_digits_ignore_us_and_czech_logical_punctuation() {
        for (code, text, mods, pane, action) in [
            (
                KeyCode::Digit1,
                "!",
                ModifiersState::SHIFT,
                0,
                Action::Jump(0, 1),
            ),
            (
                KeyCode::Digit1,
                "1",
                ModifiersState::SHIFT,
                1,
                Action::Jump(1, 1),
            ),
            (
                KeyCode::Digit1,
                "+",
                ModifiersState::CONTROL,
                0,
                Action::Jump(0, 1),
            ),
            (
                KeyCode::Digit2,
                "ě",
                ModifiersState::CONTROL,
                1,
                Action::Jump(1, 2),
            ),
            (
                KeyCode::Digit0,
                ")",
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                0,
                Action::Assign(0, 10),
            ),
            (
                KeyCode::Digit0,
                "0",
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                1,
                Action::Assign(1, 10),
            ),
            (
                KeyCode::Digit3,
                "3",
                ModifiersState::CONTROL | ModifiersState::ALT,
                0,
                Action::Jump(1, 3),
            ),
            (
                KeyCode::Digit4,
                "$",
                ModifiersState::SHIFT | ModifiersState::ALT,
                1,
                Action::Jump(0, 4),
            ),
        ] {
            let mut state = ShortcutState::default();
            modifiers(&mut state, mods);
            assert_eq!(state.key(input(code, text), pane), accepted(action));
        }
    }

    #[test]
    fn editors_popups_and_unsupported_modifiers_propagate_without_actions() {
        for pane in [-1, 2] {
            let mut state = ShortcutState::default();
            modifiers(&mut state, ModifiersState::CONTROL | ModifiersState::SHIFT);
            assert_eq!(
                state.key(input(KeyCode::Digit1, "!"), pane),
                Decision::default()
            );
        }
        for mods in [
            ModifiersState::empty(),
            ModifiersState::SUPER | ModifiersState::CONTROL,
            ModifiersState::CONTROL | ModifiersState::SHIFT | ModifiersState::ALT,
        ] {
            let mut state = ShortcutState::default();
            modifiers(&mut state, mods);
            assert_eq!(
                state.key(input(KeyCode::Digit1, "1"), 0),
                Decision::default()
            );
        }
        let mut state = ShortcutState::default();
        modifiers(&mut state, ModifiersState::CONTROL | ModifiersState::SHIFT);
        for code in [KeyCode::Equal, KeyCode::NumpadAdd, KeyCode::Enter] {
            assert_eq!(state.key(input(code, "+"), 0), Decision::default());
        }
    }

    #[test]
    fn right_alt_never_triggers_hot_paths_and_left_alt_can_target_the_other_pane() {
        let mut state = ShortcutState::default();
        modifiers(&mut state, ModifiersState::CONTROL | ModifiersState::ALT);
        let mut right_alt = input(KeyCode::AltRight, "");
        right_alt.logical = Key::Named(NamedKey::AltGraph);
        state.key(right_alt, -1);
        assert_eq!(
            state.key(input(KeyCode::Digit2, "@"), 0),
            Decision::default()
        );
        let mut release = input(KeyCode::AltRight, "");
        release.state = ElementState::Released;
        state.key(release, -1);
        state.key(input(KeyCode::AltLeft, ""), -1);
        assert_eq!(
            state.key(input(KeyCode::Digit2, "2"), 0),
            accepted(Action::Jump(1, 2))
        );
    }

    #[test]
    fn repeats_do_not_save_again_and_matched_releases_are_consumed_once() {
        let mut state = ShortcutState::default();
        modifiers(&mut state, ModifiersState::CONTROL | ModifiersState::SHIFT);
        assert_eq!(
            state.key(input(KeyCode::Digit1, "!"), 0),
            accepted(Action::Tab(0, 1))
        );
        let mut repeated = input(KeyCode::Digit1, "!");
        repeated.repeat = true;
        assert_eq!(
            state.key(repeated, 0),
            Decision {
                consume: true,
                action: None
            }
        );
        let mut release = input(KeyCode::Digit1, "!");
        release.state = ElementState::Released;
        assert_eq!(
            state.key(release, 0),
            Decision {
                consume: true,
                action: None
            }
        );
        let mut repeated = input(KeyCode::Digit1, "!");
        repeated.repeat = true;
        assert_eq!(state.key(repeated, 0), Decision::default());
        let mut synthetic = input(KeyCode::Digit1, "!");
        synthetic.synthetic = true;
        assert_eq!(state.key(synthetic, 0), Decision::default());
    }

    #[test]
    fn editor_focus_and_window_focus_loss_clear_consumption_without_stealing_input() {
        let mut state = ShortcutState::default();
        modifiers(&mut state, ModifiersState::CONTROL);
        assert_eq!(
            state.key(input(KeyCode::Digit1, "1"), 0),
            accepted(Action::Jump(0, 1))
        );
        let mut release = input(KeyCode::Digit1, "1");
        release.state = ElementState::Released;
        assert_eq!(state.key(release, -1), Decision::default());
        state.key(input(KeyCode::AltRight, ""), -1);
        state.observe(&WindowEvent::Focused(false), -1);
        assert_eq!(
            state.key(input(KeyCode::Digit1, "1"), 0),
            Decision::default()
        );
        modifiers(&mut state, ModifiersState::CONTROL);
        assert_eq!(
            state.key(input(KeyCode::Digit1, "+"), 1),
            accepted(Action::Jump(1, 1))
        );
    }

    #[test]
    fn search_or_preedit_reserves_shift_digits_but_preserves_control_and_alt_hot_paths() {
        for (mods, expected) in [
            (ModifiersState::SHIFT, Decision::default()),
            (ModifiersState::CONTROL, accepted(Action::Jump(0, 1))),
            (
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                accepted(Action::Tab(0, 1)),
            ),
            (
                ModifiersState::ALT | ModifiersState::SHIFT,
                accepted(Action::Jump(1, 1)),
            ),
        ] {
            let mut state = ShortcutState {
                search_active: true,
                ..Default::default()
            };
            modifiers(&mut state, mods);
            assert_eq!(state.key(input(KeyCode::Digit1, "!"), 0), expected);
        }
    }
}
