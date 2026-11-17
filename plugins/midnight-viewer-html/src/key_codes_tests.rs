use super::windows_key_code;
use midnight_web_viewer_api::{Key, Modifiers};

fn vk(key: &str, code: &str) -> i32 {
    windows_key_code(&Key {
        key: key.into(),
        code: code.into(),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::default(),
    })
}

/// Chromium runs navigation/editing commands (DeleteForward, MoveRight, MovePageDown,
/// context menu, F-key actions...) from these RawKeyDown virtual keys.
fn command_key(code: i32) -> bool {
    (0x08..=0x2F).contains(&code)
        || (0x5B..=0x5D).contains(&code)
        || (0x70..=0x87).contains(&code)
        || matches!(code, 0x90 | 0x91)
}

fn check(rows: &[(&str, &str, i32)]) {
    for &(key, code, expected) in rows {
        assert_eq!(vk(key, code), expected, "key {key:?} code {code:?}");
    }
}

#[test]
fn punctuation_uses_oem_keys_instead_of_navigation_or_editing_commands() {
    // US layout: "." used to become VK_DELETE, "'" VK_RIGHT, "\"" VK_NEXT and so on.
    check(&[
        (".", "Period", 0xBE),
        (">", "Period", 0xBE),
        (",", "Comma", 0xBC),
        ("'", "Quote", 0xDE),
        ("\"", "Quote", 0xDE),
        ("-", "Minus", 0xBD),
        ("_", "Minus", 0xBD),
        ("=", "Equal", 0xBB),
        ("+", "Equal", 0xBB),
        (";", "Semicolon", 0xBA),
        ("/", "Slash", 0xBF),
        ("`", "Backquote", 0xC0),
        ("[", "BracketLeft", 0xDB),
        ("{", "BracketLeft", 0xDB),
        ("\\", "Backslash", 0xDC),
        ("]", "BracketRight", 0xDD),
        ("}", "BracketRight", 0xDD),
        ("<", "IntlBackslash", 0xE2),
        ("!", "Digit1", 0x31),
        ("#", "Digit3", 0x33),
        ("$", "Digit4", 0x34),
        ("%", "Digit5", 0x35),
        ("(", "Digit9", 0x39),
        (")", "Digit0", 0x30),
    ]);
    // Czech QWERTZ: "(" is Shift+BracketRight, "%" Shift+Minus, digits sit on the shift layer.
    check(&[
        ("(", "BracketRight", 0xDD),
        ("%", "Minus", 0xBD),
        ("ě", "Digit2", 0x32),
        ("2", "Digit2", 0x32),
        ("ú", "BracketLeft", 0xDB),
        ("ů", "Semicolon", 0xBA),
        ("§", "Quote", 0xDE),
    ]);
}

#[test]
fn numpad_keys_follow_num_lock_like_windows() {
    // NumLock on: numpad virtual keys, never VK_DELETE for the decimal key.
    check(&[
        (".", "NumpadDecimal", 0x6E),
        (",", "NumpadDecimal", 0x6E),
        ("0", "Numpad0", 0x60),
        ("9", "Numpad9", 0x69),
        ("*", "NumpadMultiply", 0x6A),
        ("+", "NumpadAdd", 0x6B),
        ("-", "NumpadSubtract", 0x6D),
        ("/", "NumpadDivide", 0x6F),
        ("Enter", "NumpadEnter", 0x0D),
    ]);
    // NumLock off: the logical navigation key, exactly as Windows reports it.
    check(&[
        ("Delete", "NumpadDecimal", 0x2E),
        ("End", "Numpad1", 0x23),
        ("ArrowDown", "Numpad2", 0x28),
        ("Clear", "Numpad5", 0x0C),
        ("Insert", "Numpad0", 0x2D),
    ]);
}

#[test]
fn letters_follow_the_produced_letter_and_otherwise_the_physical_key() {
    check(&[
        ("a", "KeyA", 0x41),
        ("A", "KeyA", 0x41),
        // Czech/German QWERTZ swap Y and Z; Ctrl+Z must stay Undo there.
        ("z", "KeyY", 0x5A),
        ("y", "KeyZ", 0x59),
        // French AZERTY.
        ("a", "KeyQ", 0x41),
        ("m", "Semicolon", 0x4D),
        // Non-Latin layouts keep the physical Latin key, as Windows does.
        ("ф", "KeyA", 0x41),
        ("@", "KeyV", 0x56),
    ]);
}

#[test]
fn named_keys_are_explicit_and_unknown_keys_are_vkey_unknown() {
    check(&[
        ("Backspace", "Backspace", 0x08),
        ("Tab", "Tab", 0x09),
        ("Enter", "Enter", 0x0D),
        ("Escape", "Escape", 0x1B),
        (" ", "Space", 0x20),
        ("PageUp", "PageUp", 0x21),
        ("PageDown", "PageDown", 0x22),
        ("End", "End", 0x23),
        ("Home", "Home", 0x24),
        ("ArrowLeft", "ArrowLeft", 0x25),
        ("ArrowUp", "ArrowUp", 0x26),
        ("ArrowRight", "ArrowRight", 0x27),
        ("ArrowDown", "ArrowDown", 0x28),
        ("Insert", "Insert", 0x2D),
        ("Delete", "Delete", 0x2E),
        ("Shift", "ShiftLeft", 0x10),
        ("Control", "ControlRight", 0x11),
        ("Alt", "AltLeft", 0x12),
        ("AltGraph", "AltRight", 0x12),
        ("CapsLock", "CapsLock", 0x14),
        ("ContextMenu", "ContextMenu", 0x5D),
        ("Super", "SuperLeft", 0x5B),
        ("Super", "SuperRight", 0x5C),
        ("Meta", "MetaLeft", 0x5B),
        ("NumLock", "NumLock", 0x90),
        ("ScrollLock", "ScrollLock", 0x91),
        ("Pause", "Pause", 0x13),
        ("PrintScreen", "PrintScreen", 0x2C),
        ("F1", "F1", 0x70),
        ("F12", "F12", 0x7B),
        ("F24", "F24", 0x87),
        ("€", "Unidentified", 0),
        ("日", "Unidentified", 0),
        ("MediaPlayPause", "MediaPlayPause", 0),
        ("Unidentified", "Unidentified", 0),
    ]);
}

#[test]
fn no_printable_symbol_reaches_a_command_key_on_any_physical_key() {
    let mut codes: Vec<String> = [
        "Backquote",
        "Backslash",
        "BracketLeft",
        "BracketRight",
        "Comma",
        "Equal",
        "IntlBackslash",
        "IntlRo",
        "IntlYen",
        "Minus",
        "Period",
        "Quote",
        "Semicolon",
        "Slash",
        "NumpadAdd",
        "NumpadComma",
        "NumpadDecimal",
        "NumpadDivide",
        "NumpadEqual",
        "NumpadMultiply",
        "NumpadSubtract",
        "Unidentified",
    ]
    .iter()
    .map(|code| code.to_string())
    .collect();
    codes.extend((0..10).map(|digit| format!("Digit{digit}")));
    codes.extend((0..10).map(|digit| format!("Numpad{digit}")));
    codes.extend(('A'..='Z').map(|letter| format!("Key{letter}")));
    let symbols = (0x21u8..=0x7E)
        .map(char::from)
        .filter(|c| !c.is_ascii_alphanumeric())
        .map(String::from)
        .chain(["ě", "ů", "§", "€", "ß", "日"].map(String::from));
    for symbol in symbols {
        for code in &codes {
            let key = vk(&symbol, code);
            assert!(
                !command_key(key),
                "{symbol:?} on {code} became command VK {key:#04x}"
            );
        }
    }
}
