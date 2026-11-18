//! Displayed text of file and folder names and paths.
//!
//! A name may contain the explicit bidirectional embedding, override and isolate controls
//! (U+202A-U+202E, U+2066-U+2069). Rendered as they are, they reorder the visible text,
//! so that `photo` + U+202E + `gpj.exe` shows as `photoexe.jpg`. Displayed names and paths
//! show each of them as a visible `\u202E`-style escape instead, the form the Basic viewer
//! and the text comparator use. The escape names the code point, so nothing is lost. The
//! implicit direction marks (LRM, RLM, ALM) stay: they cannot open a directional run, and
//! right-to-left text needs them. Only displayed text passes through here; raw names and
//! paths remain the identities for operations, selection, search, comparisons and the
//! clipboard.
use std::{borrow::Cow, fmt::Write};

/// Whether `c` is an explicit bidirectional embedding, override or isolate control.
pub fn is_explicit_bidi_control(c: char) -> bool {
    matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// `text` as a window shows it: each explicit bidirectional control becomes a visible
/// escape such as `\u202E`. Text without such controls is returned unchanged.
pub fn visible_name(text: &str) -> Cow<'_, str> {
    // The UTF-8 encoding of every such control starts with 0xE2; most text has no such
    // byte, and this check needs no character decoding.
    if !text.as_bytes().contains(&0xe2) || !text.contains(is_explicit_bidi_control) {
        return Cow::Borrowed(text);
    }
    let mut shown = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        if is_explicit_bidi_control(c) {
            // Writing to a String cannot fail.
            let _ = write!(shown, "\\u{:04X}", u32::from(c));
        } else {
            shown.push(c);
        }
    }
    Cow::Owned(shown)
}

/// The byte offset in `visible_name(text)` of the byte offset `offset` in `text`: each
/// escaped control before it shows as six ASCII bytes instead of its three UTF-8 bytes.
/// An offset beyond the end of `text` is treated as its end.
pub fn visible_name_offset(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    let escaped = text
        .char_indices()
        .take_while(|(index, _)| *index < offset)
        .filter(|(_, c)| is_explicit_bidi_control(*c))
        .count();
    offset + 3 * escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_bidi_controls_are_escaped_and_implicit_marks_kept() {
        assert_eq!(
            visible_name("photo\u{202e}gpj.exe"),
            "photo\\u202Egpj.exe",
            "the extension cannot be reordered"
        );
        assert_eq!(
            visible_name("a\u{202a}b\u{2066}c\u{2069}d\u{200e}e\u{200f}f\u{61c}g"),
            "a\\u202Ab\\u2066c\\u2069d\u{200e}e\u{200f}f\u{61c}g"
        );
        for control in ('\u{202a}'..='\u{202e}').chain('\u{2066}'..='\u{2069}') {
            assert!(is_explicit_bidi_control(control));
            let shown = visible_name(&format!("x{control}y")).into_owned();
            assert_eq!(shown, format!("x\\u{:04X}y", u32::from(control)));
            assert!(!shown.contains(is_explicit_bidi_control));
        }
        for kept in [
            '\u{200e}', '\u{200f}', '\u{61c}', '\u{2065}', '\u{2029}', '—',
        ] {
            assert!(!is_explicit_bidi_control(kept));
        }
    }

    #[test]
    fn text_without_controls_is_borrowed_unchanged() {
        for text in [
            "",
            "plain.txt",
            "资料 — עברית e\u{301} 🧪",
            "C:\\u202E literal",
        ] {
            assert!(matches!(visible_name(text), Cow::Borrowed(shown) if shown == text));
        }
    }

    #[test]
    fn offsets_follow_the_escapes_before_them() {
        let text = "ab\u{202e}cd\u{2067}e";
        let shown = visible_name(text);
        assert_eq!(shown, "ab\\u202Ecd\\u2067e");
        for (raw, expected) in [
            (0, 0),
            (2, 2),
            (2 + 3, 2 + 6),
            (2 + 3 + 2, 2 + 6 + 2),
            (text.len(), shown.len()),
            (text.len() + 7, shown.len()),
        ] {
            assert_eq!(visible_name_offset(text, raw), expected, "offset {raw}");
        }
        assert_eq!(visible_name_offset("plain", 3), 3);
    }
}
