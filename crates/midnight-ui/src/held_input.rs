//! Keys typed into a panel that waits for a file job's change
//! (`left-input-held`, `right-input-held`) wait in order and are replayed
//! once the wait ends (the panel shows the change, the window's limit for its
//! re-read expires, or Escape ends it), so no key acts on stale rows and none
//! is lost. Only the held panel's file list waits: keys for the other panel,
//! the command line or a dialog pass at once.
use super::{Bindings, physical_shortcut};
use crate::MainWindow;
use i_slint_core::input::{KeyEvent, KeyboardModifiers};
use midnight_core::ShortcutChord;
use slint::{
    ComponentHandle, SharedString,
    platform::{Key, WindowEvent},
};
use std::{cell::RefCell, collections::VecDeque, mem, rc::Rc};

/// Keys a held panel keeps, a few seconds of fast typing; later ones are
/// swallowed rather than acted on stale rows.
const MAX_HELD: usize = 64;

enum Held {
    Key {
        event: KeyEvent,
        pressed: bool,
        /// `native-digit-event` as the key was typed: a number-row key is text,
        /// never a logical shortcut. Replayed keys run with their own value.
        digit: bool,
    },
    /// A shortcut the native window hook takes before Slint sees its key (number
    /// row, keypad + − *, brackets), held whether or not it is bound: the earlier
    /// keys it waits behind can change what it means (a started quick search).
    /// Replayed when no shortcut takes it any more, it types `text` instead.
    Physical {
        chord: ShortcutChord,
        text: SharedString,
    },
}

#[derive(Default)]
pub(super) struct HeldInput {
    /// The panel whose keys wait.
    pane: i32,
    events: VecDeque<Held>,
    /// The modifiers Slint stamped on the last key event outside a replay:
    /// the keys held down now.
    live: KeyboardModifiers,
    replaying: bool,
}

pub(super) type State = Rc<RefCell<HeldInput>>;

pub(super) fn install(window: &MainWindow, state: State, bindings: Bindings) {
    let weak = window.as_weak();
    let held = state.clone();
    window.on_hold_key(move |event, pressed| {
        weak.upgrade()
            .is_some_and(|window| hold_key(&window, &held, event, pressed))
    });
    let weak = window.as_weak();
    window.on_release_held_keys(move || {
        if let Some(window) = weak.upgrade() {
            replay(&window, &state, &bindings);
        }
    });
}

fn input_held(window: &MainWindow, pane: i32) -> bool {
    match pane {
        0 => window.get_left_input_held(),
        1 => window.get_right_input_held(),
        _ => false,
    }
}

fn is_modifier(text: &str) -> bool {
    [
        Key::Control,
        Key::ControlR,
        Key::Shift,
        Key::ShiftR,
        Key::Alt,
        Key::AltGr,
        Key::Meta,
        Key::MetaR,
    ]
    .into_iter()
    .any(|key| text == slint::SharedString::from(key).as_str())
}

/// The panel whose file list would receive input now, when that input waits:
/// the panel is held, or earlier keys typed into it still wait.
fn waiting_pane(window: &MainWindow, state: &HeldInput) -> Option<i32> {
    if state.replaying || window.get_dialog_input_blocked() || window.get_modal_overlay_open() {
        return None;
    }
    let pane = window.get_focused_pane();
    (pane >= 0 && (input_held(window, pane) || (!state.events.is_empty() && state.pane == pane)))
        .then_some(pane)
}

fn push(window: &MainWindow, state: &mut HeldInput, pane: i32, held: Held) {
    if state.pane != pane {
        // Keys left by a panel the user clicked away from target rows of that
        // panel; replaying them into another one would act on the wrong rows.
        state.events.clear();
        state.pane = pane;
    }
    if state.events.len() < MAX_HELD {
        state.events.push_back(held);
    }
    window.set_held_keys_pending(true);
}

/// Called for every key event the main window's shortcuts scope captures.
fn hold_key(window: &MainWindow, state: &State, event: KeyEvent, pressed: bool) -> bool {
    let mut held = state.borrow_mut();
    if !held.replaying {
        held.live = event.modifiers;
    }
    // A modifier changes no rows; it passes so the footer and later keys see
    // it, and each replayed key restores the modifiers it was typed with.
    if is_modifier(event.text.as_str()) {
        return false;
    }
    let Some(pane) = waiting_pane(window, &held) else {
        return false;
    };
    // Escape ends the panel's wait for the user; the keys typed before it are
    // replayed once the window sees the wait ended.
    if pressed
        && event.text.as_str() == slint::SharedString::from(Key::Escape).as_str()
        && input_held(window, pane)
    {
        match pane {
            0 => window.set_left_hold_released(true),
            _ => window.set_right_hold_released(true),
        }
        return true;
    }
    // A release acts on no rows; it waits only to follow its waiting press.
    let follows_press = held.pane == pane
        && held.events.iter().any(|waiting| match waiting {
            Held::Key {
                event: press,
                pressed: true,
                ..
            } => press.text == event.text,
            _ => false,
        });
    if !pressed && !follows_press {
        return false;
    }
    let digit = window.get_native_digit_event();
    push(
        window,
        &mut held,
        pane,
        Held::Key {
            event,
            pressed,
            digit,
        },
    );
    true
}

/// A held panel's native-hook shortcut (digit row, keypad, brackets) waits with
/// its keys, together with the text its key types (`native-key-text`).
pub(super) fn hold_physical(window: &MainWindow, state: &State, chord: &ShortcutChord) -> bool {
    let mut held = state.borrow_mut();
    let Some(pane) = waiting_pane(window, &held) else {
        return false;
    };
    let text = window.get_native_key_text();
    push(
        window,
        &mut held,
        pane,
        Held::Physical {
            chord: chord.clone(),
            text,
        },
    );
    true
}

/// Replay the waiting keys once their panel's hold ended (or expired), in the
/// order they were typed and with the modifiers they were typed with. Keys of
/// a panel that lost focus meanwhile (a click elsewhere, a dialog) are dropped
/// rather than delivered to another target.
fn replay(window: &MainWindow, state: &State, bindings: &Bindings) {
    let (pane, events, live) = {
        let mut held = state.borrow_mut();
        if held.replaying || held.events.is_empty() || input_held(window, held.pane) {
            return;
        }
        held.replaying = true;
        (held.pane, mem::take(&mut held.events), held.live)
    };
    window.set_held_keys_pending(false);
    let delivered = window.get_focused_pane() == pane
        && !window.get_dialog_input_blocked()
        && !window.get_embedded_modal_open();
    if delivered {
        let mut current = live;
        // The hook's flag describes its latest native key, not a replayed one.
        let live_digit = window.get_native_digit_event();
        for held in events {
            match held {
                Held::Key {
                    event,
                    pressed,
                    digit,
                } => {
                    align_modifiers(window, &mut current, event.modifiers);
                    window.set_native_digit_event(digit);
                    let text = event.text.clone();
                    window.window().dispatch_event(if !pressed {
                        WindowEvent::KeyReleased { text }
                    } else if event.repeat {
                        WindowEvent::KeyPressRepeated { text }
                    } else {
                        WindowEvent::KeyPressed { text }
                    });
                }
                // The chord carries its own modifiers. Unbound now (say, a digit
                // after letters that started a quick search), its key types its
                // text as it would have without the wait: a number-row key as
                // text, never as a logical shortcut.
                Held::Physical { chord, text } => {
                    if !physical_shortcut(window, bindings, &chord) && !text.is_empty() {
                        let mut typed = KeyboardModifiers::default();
                        typed.control = chord.control;
                        typed.alt = chord.alt;
                        typed.shift = chord.shift;
                        typed.meta = chord.meta;
                        align_modifiers(window, &mut current, typed);
                        window.set_native_digit_event(chord.key.starts_with("Digit"));
                        let window = window.window();
                        window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                        window.dispatch_event(WindowEvent::KeyReleased { text });
                    }
                }
            }
        }
        align_modifiers(window, &mut current, live);
        window.set_native_digit_event(live_digit);
    }
    let mut held = state.borrow_mut();
    held.replaying = false;
    window.set_held_keys_pending(!held.events.is_empty());
}

/// Bring Slint's modifier state (`current`) to `wanted` with synthetic
/// modifier key events: Slint stamps each key event with the modifiers it
/// tracked from earlier key events, not with those the key was typed with.
fn align_modifiers(
    window: &MainWindow,
    current: &mut KeyboardModifiers,
    mut wanted: KeyboardModifiers,
) {
    type Flag = fn(&mut KeyboardModifiers) -> &mut bool;
    let modifiers: [(Key, Option<Key>, Flag); 4] = [
        (Key::Control, Some(Key::ControlR), |m| &mut m.control),
        (Key::Shift, Some(Key::ShiftR), |m| &mut m.shift),
        (Key::Alt, None, |m| &mut m.alt),
        (Key::Meta, Some(Key::MetaR), |m| &mut m.meta),
    ];
    for (key, right, flag) in modifiers {
        let target = *flag(&mut wanted);
        if *flag(current) == target {
            continue;
        }
        let window = window.window();
        if target {
            window.dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        } else {
            window.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
            if let Some(right) = right {
                window.dispatch_event(WindowEvent::KeyReleased { text: right.into() });
            }
        }
        *flag(current) = target;
    }
}
