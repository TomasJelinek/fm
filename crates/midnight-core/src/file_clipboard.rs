//! Untrusted native file-list clipboard data, captured on the backend.

use crate::FsError;
use std::{
    io,
    path::{Path, PathBuf},
};

pub const MAX_FILE_CLIPBOARD_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_FILE_CLIPBOARD_PATHS: usize = 4096;

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
    pub fn validate(&self) -> Result<(), FsError> {
        let invalid = |message| {
            FsError::from_io(
                "file clipboard",
                Path::new(""),
                io::Error::new(io::ErrorKind::InvalidInput, message),
            )
        };
        if self.paths.is_empty() || self.paths.len() > MAX_FILE_CLIPBOARD_PATHS {
            return Err(invalid(
                "A file clipboard requires between 1 and 4096 paths.",
            ));
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
