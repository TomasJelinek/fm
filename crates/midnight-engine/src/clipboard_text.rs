//! Literal path-paste policy; native clipboard access remains in platform services.
use std::sync::Arc;

pub(crate) const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

pub(crate) fn prepare_pasted_path(mut text: String) -> Result<Arc<str>, &'static str> {
    if text.len() > MAX_TEXT_BYTES {
        return Err("Clipboard text exceeds its 4 MiB operation resource budget.");
    }
    if text.contains(['\0', '\r', '\n']) {
        return Err(
            "Clipboard text contains NUL or a line break and cannot be pasted as a literal single-line path.",
        );
    }
    if !text.is_empty() && !text.ends_with(std::path::is_separator) {
        text.push(std::path::MAIN_SEPARATOR);
    }
    Ok(Arc::from(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pasted_path_keeps_literal_text_and_native_termination() {
        let separator = std::path::MAIN_SEPARATOR;
        for text in [
            "资料/e\u{301}",
            "relative folder ",
            "\"quoted path\"",
            "..",
            "private\u{f708}🦀\tpath",
        ] {
            assert_eq!(
                &*prepare_pasted_path(text.into()).unwrap(),
                format!("{text}{separator}")
            );
        }
        for text in ["", "/", "folder/"] {
            assert_eq!(&*prepare_pasted_path(text.into()).unwrap(), text);
        }
        let expected = if std::path::is_separator('\\') {
            "folder\\".into()
        } else {
            format!("folder\\{separator}")
        };
        assert_eq!(&*prepare_pasted_path("folder\\".into()).unwrap(), expected);
    }
    #[test]
    fn paste_refuses_unrepresentable_single_line_text_and_oversized_payloads() {
        for text in ["first\nsecond", "first\rsecond", "first\0second"] {
            assert!(prepare_pasted_path(text.into()).is_err());
        }
        assert!(prepare_pasted_path("x".repeat(MAX_TEXT_BYTES + 1)).is_err());
    }
}
