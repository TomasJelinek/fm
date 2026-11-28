//! Untrusted native file-list clipboard data, captured on the backend.

use crate::FsError;
use std::{
    io,
    path::{Path, PathBuf},
};

/// Bytes of native paths that one file list holds at most: the memory bound on a native
/// clipboard or drag-and-drop payload, which another program may supply. It carries
/// `MAX_NATIVE_TRANSFER_ROOTS` paths of about 130 UTF-16 characters (Windows) or 260 bytes
/// (Linux URI lists). Each path takes at least a few of them, so this also bounds how many
/// paths a list holds; the native formats themselves (CF_HDROP, URI lists) have no count
/// limit, and neither has this.
pub const MAX_FILE_CLIPBOARD_BYTES: usize = 256 * 1024 * 1024;
/// Native items that one Copy, Move or drop takes at most. A job keeps only the selected
/// paths and a few bytes per item (its identity and kind, captured in one pass that closes
/// each item again) and opens each item just before copying it, so the bound is the memory
/// of that list, about 200 bytes per item. Clipboard and drag payloads themselves are
/// bounded only by `MAX_FILE_CLIPBOARD_BYTES`. Archive members are exempt.
pub const MAX_NATIVE_TRANSFER_ROOTS: usize = 1_000_000;
/// `MAX_NATIVE_TRANSFER_ROOTS` in the words of its refusals.
pub const MAX_NATIVE_TRANSFER_ROOTS_TEXT: &str = "1,000,000";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileClipboardIntent {
    Copy,
    Move,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileClipboard {
    pub paths: Vec<PathBuf>,
    pub intent: FileClipboardIntent,
}

impl FileClipboard {
    /// Reject malformed lists whole. Adapters additionally bound their encoded native payload.
    /// This checks syntax only; the operation backend must validate current filesystem identities.
    /// Any number of paths within `MAX_FILE_CLIPBOARD_BYTES` is valid: consumers that hold a
    /// resource per path bound their own count (`MAX_NATIVE_TRANSFER_ROOTS` for transfers).
    pub fn validate(&self) -> Result<(), FsError> {
        let invalid = |message| {
            FsError::from_io(
                "file clipboard",
                Path::new(""),
                io::Error::new(io::ErrorKind::InvalidInput, message),
            )
        };
        if self.paths.is_empty() {
            return Err(invalid("A file clipboard requires at least one path."));
        }
        let mut bytes = 0usize;
        for path in &self.paths {
            let native = path.as_os_str().as_encoded_bytes();
            if !path.is_absolute() || native.contains(&0) {
                return Err(invalid(
                    "File clipboard paths must be absolute and contain no NUL.",
                ));
            }
            bytes = bytes.saturating_add(native.len());
            if bytes > MAX_FILE_CLIPBOARD_BYTES {
                return Err(invalid(
                    "File clipboard paths exceed their 256 MiB resource budget.",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusal_texts_name_the_bounds() {
        let digits: String = MAX_NATIVE_TRANSFER_ROOTS_TEXT
            .chars()
            .filter(char::is_ascii_digit)
            .collect();
        assert_eq!(digits, MAX_NATIVE_TRANSFER_ROOTS.to_string());
        assert_eq!(MAX_FILE_CLIPBOARD_BYTES, 256 << 20);
    }
}
