//! Linux clipboard ownership is retained on the backend until shutdown.
use midnight_core::{ClipboardService, ClipboardSession, FileClipboard, FsError};
use std::{io, path::Path};

#[derive(Default)]
pub struct NativeClipboard;

struct Session {
    text: arboard::Clipboard,
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
        arboard::Clipboard::new()
            .map(|text| Box::new(Session { text, files: None }) as Box<dyn ClipboardSession>)
            .map_err(failure)
    }
}

impl ClipboardSession for Session {
    fn copy_text(&mut self, text: &str) -> Result<(), FsError> {
        self.text.set_text(text).map_err(failure)
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
