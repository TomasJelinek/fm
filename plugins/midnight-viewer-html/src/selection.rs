//! Explicit clipboard-budget state; an overflow must never become empty clipboard text.
use midnight_web_viewer_api::{ErrorCode, EventKind};

const MAX_SELECTION_BYTES: usize = 48 * 1024;

#[derive(Default)]
pub(crate) enum Selection {
    #[default]
    Empty,
    Text(String),
    Overflow,
}

impl Selection {
    pub(crate) fn capture(&mut self, utf16: &[u16]) {
        *self = if utf16.len() > MAX_SELECTION_BYTES {
            Self::Overflow
        } else {
            // UTF-16 length bounds the conversion allocation first. UTF-8 may then
            // occupy more bytes, so check its actual encoded length separately.
            let text = String::from_utf16_lossy(utf16);
            if text.len() > MAX_SELECTION_BYTES {
                Self::Overflow
            } else {
                Self::Text(text)
            }
        };
    }

    pub(crate) fn copy_event(&self) -> EventKind {
        match self {
            Self::Empty => EventKind::Selection(String::new()),
            Self::Text(text) => EventKind::Selection(text.clone()),
            Self::Overflow => EventKind::Error {
                code: ErrorCode::BudgetExceeded,
                message: "Selected text exceeds the 48 KiB copy budget; select a smaller range."
                    .into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_selection_rejects_copy_and_valid_selection_recovers_without_truncation() {
        let mut selection = Selection::default();
        selection.capture(&vec![u16::from(b'x'); MAX_SELECTION_BYTES + 1]);
        assert!(matches!(
            selection.copy_event(),
            EventKind::Error {
                code: ErrorCode::BudgetExceeded,
                ..
            }
        ));
        let unicode: Vec<_> = "\u{1f600}".repeat(16 * 1024).encode_utf16().collect();
        selection.capture(&unicode);
        assert!(matches!(
            selection.copy_event(),
            EventKind::Error {
                code: ErrorCode::BudgetExceeded,
                ..
            }
        ));
        let valid = "x".repeat(40 * 1024);
        selection.capture(&valid.encode_utf16().collect::<Vec<_>>());
        assert_eq!(selection.copy_event(), EventKind::Selection(valid));
        selection.capture(&[]);
        assert_eq!(selection.copy_event(), EventKind::Selection(String::new()));
    }
}
