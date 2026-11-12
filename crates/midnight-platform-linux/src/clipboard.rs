//! Linux clipboard ownership is retained on the backend until shutdown.
use midnight_core::{ClipboardService, ClipboardSession, FileClipboard, FsError};
use std::{io, path::Path};

#[derive(Default)]
pub struct NativeClipboard;

#[derive(Default)]
struct Session {
    text: Option<arboard::Clipboard>,
    files: Option<crate::file_clipboard::Files>,
}

impl Session {
    fn files(&mut self) -> Result<&mut crate::file_clipboard::Files, FsError> {
        if self.files.is_none() {
            self.files = Some(crate::file_clipboard::Files::new()?);
        }
        Ok(self.files.as_mut().expect("file clipboard was initialized"))
    }
}

impl ClipboardService for NativeClipboard {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        // Bounded text/file reads choose their own native transport. Do not
        // require an unrelated clipboard owner or X11 connection beforehand.
        Ok(Box::new(Session::default()))
    }
}

impl ClipboardSession for Session {
    fn read_text(&mut self) -> Result<String, FsError> {
        crate::clipboard_text::read(self.files()?)
    }
    fn copy_text(&mut self, text: &str) -> Result<(), FsError> {
        if self.text.is_none() {
            self.text = Some(arboard::Clipboard::new().map_err(failure)?);
        }
        self.text
            .as_mut()
            .expect("text clipboard initialized")
            .set_text(text)
            .map_err(failure)
    }
    fn copy_files(&mut self, files: &FileClipboard) -> Result<(), FsError> {
        self.files()?.copy(files)
    }
    fn read_files(&mut self) -> Result<FileClipboard, FsError> {
        self.files()?.read()
    }
    fn service_pending(&mut self) -> Result<(), FsError> {
        if let Some(files) = &mut self.files {
            files.service_pending()?;
        }
        Ok(())
    }
}

fn failure(error: arboard::Error) -> FsError {
    FsError::from_io(
        "Linux clipboard",
        Path::new(""),
        io::Error::other(error.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_open_does_not_require_a_display_or_text_owner() {
        // Also run with DISPLAY unset and an isolated, empty Wayland runtime.
        // Read transport selection owns capability errors, not session construction.
        assert!(NativeClipboard.open().is_ok());
    }
}
