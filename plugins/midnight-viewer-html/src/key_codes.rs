//! Platform-neutral Windows virtual-key derivation for CEF `windows_key_code`.
//!
//! Chromium executes editing and navigation commands (DeleteForward, MoveRight,
//! MovePageDown...) from the RawKeyDown virtual key, while the typed character
//! arrives separately as Text. A printable key must therefore never produce a
//! navigation/editing VK: punctuation maps by its physical key to the US-layout
//! OEM key (as Chromium's DOM-code fallback does), never through its ASCII value.
use midnight_web_viewer_api as api;

/// Returns the Windows `VK_*` value for a W3C key/code pair, or 0 (VKEY_UNKNOWN).
pub(crate) fn windows_key_code(key: &api::Key) -> i32 {
    let logical = key.key.as_str();
    let code = key.code.as_str();
    // Named logical keys first: with NumLock off a numpad key reports its navigation
    // meaning ("End", "Delete"), exactly as the Windows VK does.
    if let Some(value) = named(logical, code) {
        return value;
    }
    if let Some(value) = numpad(code) {
        return value;
    }
    let mut characters = logical.chars();
    let single = characters.next().filter(|_| characters.next().is_none());
    // Windows letter VKs follow the layout (QWERTZ Y/Z, AZERTY A/Q, French M), so
    // Ctrl+Z stays Undo; winit's logical key ignores Ctrl.
    if let Some(letter) = single.filter(char::is_ascii_alphabetic) {
        return i32::from(letter.to_ascii_uppercase() as u8);
    }
    if let Some(value) = physical(code) {
        return value;
    }
    single.and_then(us_character).unwrap_or(0)
}

fn named(logical: &str, code: &str) -> Option<i32> {
    if let Some(index) = logical
        .strip_prefix('F')
        .and_then(|digits| digits.parse::<i32>().ok())
        .filter(|index| (1..=24).contains(index))
    {
        return Some(0x6F + index);
    }
    Some(match logical {
        "Backspace" => 0x08,
        "Tab" => 0x09,
        "Clear" => 0x0C,
        "Enter" => 0x0D,
        "Shift" => 0x10,
        "Control" => 0x11,
        // Windows reports the right Alt (AltGr) key as VK_MENU.
        "Alt" | "AltGraph" => 0x12,
        "Pause" => 0x13,
        "CapsLock" => 0x14,
        "KanaMode" | "HangulMode" => 0x15,
        "HanjaMode" | "KanjiMode" => 0x19,
        "Escape" => 0x1B,
        "Convert" => 0x1C,
        "NonConvert" => 0x1D,
        " " | "Space" => 0x20,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "End" => 0x23,
        "Home" => 0x24,
        "ArrowLeft" => 0x25,
        "ArrowUp" => 0x26,
        "ArrowRight" => 0x27,
        "ArrowDown" => 0x28,
        "Select" => 0x29,
        "PrintScreen" => 0x2C,
        "Insert" => 0x2D,
        "Delete" => 0x2E,
        "Help" => 0x2F,
        "Meta" | "Super" | "OS" if code.ends_with("Right") => 0x5C,
        "Meta" | "Super" | "OS" => 0x5B,
        "ContextMenu" => 0x5D,
        "NumLock" => 0x90,
        "ScrollLock" => 0x91,
        // Keys consumed by an input method.
        "Process" => 0xE5,
        _ => return None,
    })
}

/// Numpad keys whose logical key is a character (NumLock on, or operators).
fn numpad(code: &str) -> Option<i32> {
    if let Some(digit) = code
        .strip_prefix("Numpad")
        .filter(|digit| digit.len() == 1)
        .and_then(|digit| digit.parse::<i32>().ok())
    {
        return Some(0x60 + digit);
    }
    Some(match code {
        "NumpadMultiply" => 0x6A,
        "NumpadAdd" => 0x6B,
        "NumpadSubtract" => 0x6D,
        "NumpadDecimal" => 0x6E,
        "NumpadDivide" => 0x6F,
        "NumpadEnter" => 0x0D,
        "NumpadComma" => 0xBC,
        _ => return None,
    })
}

/// US-layout virtual key of a physical key (Chromium's DomCode fallback table).
fn physical(code: &str) -> Option<i32> {
    if let Some(letter) = code
        .strip_prefix("Key")
        .and_then(|letter| letter.parse::<char>().ok())
        .filter(char::is_ascii_uppercase)
    {
        return Some(i32::from(letter as u8));
    }
    if let Some(digit) = code
        .strip_prefix("Digit")
        .and_then(|digit| digit.parse::<char>().ok())
        .filter(char::is_ascii_digit)
    {
        return Some(i32::from(digit as u8));
    }
    Some(match code {
        "Semicolon" => 0xBA,
        "Equal" => 0xBB,
        "Comma" => 0xBC,
        "Minus" => 0xBD,
        "Period" => 0xBE,
        "Slash" => 0xBF,
        "Backquote" => 0xC0,
        "BracketLeft" => 0xDB,
        "Backslash" | "IntlYen" => 0xDC,
        "BracketRight" => 0xDD,
        "Quote" => 0xDE,
        "IntlBackslash" | "IntlRo" => 0xE2,
        _ => return None,
    })
}

/// Last resort for an unidentified physical key: the US-layout key producing the
/// ASCII character. Anything else is VKEY_UNKNOWN; the text still arrives as Text.
fn us_character(character: char) -> Option<i32> {
    Some(match character {
        '0'..='9' => i32::from(character as u8),
        ')' => 0x30,
        '!' => 0x31,
        '@' => 0x32,
        '#' => 0x33,
        '$' => 0x34,
        '%' => 0x35,
        '^' => 0x36,
        '&' => 0x37,
        '*' => 0x38,
        '(' => 0x39,
        ';' | ':' => 0xBA,
        '=' | '+' => 0xBB,
        ',' | '<' => 0xBC,
        '-' | '_' => 0xBD,
        '.' | '>' => 0xBE,
        '/' | '?' => 0xBF,
        '`' | '~' => 0xC0,
        '[' | '{' => 0xDB,
        '\\' | '|' => 0xDC,
        ']' | '}' => 0xDD,
        '\'' | '"' => 0xDE,
        _ => return None,
    })
}

#[cfg(test)]
#[path = "key_codes_tests.rs"]
mod tests;
