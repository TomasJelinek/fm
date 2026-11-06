//! Linux clipboard ownership is retained on the backend until shutdown.
use midnight_core::{ClipboardService, ClipboardSession, FsError};
use std::{io, path::Path};

#[derive(Default)]
pub struct NativeClipboard;

struct Session(arboard::Clipboard);

impl ClipboardService for NativeClipboard {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        arboard::Clipboard::new()
            .map(|clipboard| Box::new(Session(clipboard)) as Box<dyn ClipboardSession>)
            .map_err(failure)
    }
}

impl ClipboardSession for Session {
    fn copy_text(&mut self, text: &str) -> Result<(), FsError> {
        self.0.set_text(text).map_err(failure)
    }
}

fn failure(error: arboard::Error) -> FsError {
    FsError::from_io(
        "Linux clipboard",
        Path::new(""),
        io::Error::other(error.to_string()),
    )
}
