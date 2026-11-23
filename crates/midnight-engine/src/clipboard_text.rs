//! Literal path-paste policy; native clipboard access remains in platform services.
use std::sync::Arc;

pub(crate) const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

/// Checks clipboard text for the path editor and returns it exactly as copied: no
/// separator, quote, whitespace or letter case is added or removed.
pub(crate) fn prepare_pasted_path(text: String) -> Result<Arc<str>, &'static str> {
    if text.len() > MAX_TEXT_BYTES {
        return Err("Clipboard text exceeds its 4 MiB operation resource budget.");
    }
    if text.contains(['\0', '\r', '\n']) {
        return Err(
            "Clipboard text contains NUL or a line break and cannot be pasted as a literal single-line path.",
        );
    }
    Ok(Arc::from(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pasted_path_keeps_exactly_the_copied_text() {
        for text in [
            "",
            "资料/e\u{301}",
            "relative folder ",
            "\"quoted path\"",
            "..",
            "private\u{f708}🦀\tpath",
            "/",
            "folder/",
            "folder\\",
            r"C:\Users\me\Documents",
            r"C:\Users\me\Documents\",
            "C:/Users/me/Documents",
            "C:/Users/me/Documents/",
            r"\\server\share\folder",
            r"\\server\share\folder\",
            r"\\?\C:\very\long",
            "/home/me/docs",
            "/home/me/docs/",
            "/home/me/report.txt",
        ] {
            assert_eq!(&*prepare_pasted_path(text.into()).unwrap(), text);
        }
    }
    #[test]
    fn paste_refuses_unrepresentable_single_line_text_and_oversized_payloads() {
        for text in ["first\nsecond", "first\rsecond", "first\0second"] {
            assert!(prepare_pasted_path(text.into()).is_err());
        }
        assert!(prepare_pasted_path("x".repeat(MAX_TEXT_BYTES + 1)).is_err());
    }
}
