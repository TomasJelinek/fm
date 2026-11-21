//! Untrusted native file-list clipboard data, captured on the backend.

use crate::FsError;
use std::{
    io,
    path::{Path, PathBuf},
};

/// Bytes of native paths that one file list holds at most: the memory bound on a native
/// clipboard or drag-and-drop payload, which another program may supply. Each path takes
/// at least a few of them, so this also bounds how many paths a list holds; the native
/// formats themselves (CF_HDROP, URI lists) have no count limit, and neither has this.
pub const MAX_FILE_CLIPBOARD_BYTES: usize = 4 * 1024 * 1024;
/// Native roots that one Copy, Move or drop takes at most. Each root keeps its source
/// and parent folder open (two handles or descriptors) from preparation until it is
/// copied, and every root is pinned when the job starts; on Linux these count against
/// the process's open-file limit. Clipboard and drag payloads themselves are bounded only
/// by `MAX_FILE_CLIPBOARD_BYTES`. Archive members hold no native handle and are exempt.
pub const MAX_NATIVE_TRANSFER_ROOTS: usize = 4096;

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
                    "File clipboard paths exceed their 4 MiB resource budget.",
                ));
            }
        }
        Ok(())
    }
}
