//! Canonicalize SDK input without deriving physical digits from keyboard text.
use i_slint_core::input::KeyEvent;
use midnight_core::ShortcutChord;
use slint::platform::Key;

pub fn chord(event: &KeyEvent) -> Option<ShortcutChord> {
    let mut chars = event.text.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let named = [
        (Key::Return, "Enter"),
        (Key::Escape, "Escape"),
        (Key::Tab, "Tab"),
        (Key::Backspace, "Backspace"),
        (Key::Delete, "Delete"),
        (Key::Insert, "Insert"),
        (Key::Home, "Home"),
        (Key::End, "End"),
        (Key::PageUp, "PageUp"),
        (Key::PageDown, "PageDown"),
        (Key::UpArrow, "Up"),
        (Key::DownArrow, "Down"),
        (Key::LeftArrow, "Left"),
        (Key::RightArrow, "Right"),
        (Key::Space, "Space"),
    ]
    .into_iter()
    .find_map(|(key, name)| (char::from(key) == first).then_some(name));
    let functions = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
        Key::F13,
        Key::F14,
        Key::F15,
        Key::F16,
        Key::F17,
        Key::F18,
        Key::F19,
        Key::F20,
        Key::F21,
        Key::F22,
        Key::F23,
        Key::F24,
    ];
    let key = if let Some(named) = named {
        named.to_owned()
    } else if let Some(index) = functions
        .into_iter()
        .position(|key| char::from(key) == first)
    {
        format!("F{}", index + 1)
    } else if first == '+' {
        "Plus".into()
    } else {
        first.to_string()
    };
    let mut chord = ShortcutChord::parse(&key).ok()?;
    chord.control = event.modifiers.control;
    chord.alt = event.modifiers.alt;
    chord.shift = event.modifiers.shift;
    chord.meta = event.modifiers.meta;
    Some(chord)
}
