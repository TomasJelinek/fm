//! Generic web-viewer transport and ownership helpers.
#![forbid(unsafe_code)]

mod queue;
mod session;

pub use queue::{CommandQueue, FrameWindow, MAX_QUEUED_COMMANDS, MAX_RETAINED_FRAMES};
pub use session::Session;

use midnight_web_viewer_api::{Message, read_message, write_message};
use std::io::{self, Read, Write};

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid web-viewer session transition",
    )
}

/// Blocking generic I/O; use on a backend worker, never an application UI callback.
/// Any receive/send failure poisons both directions. Drop it and release session ownership.
pub struct Transport<R, W> {
    reader: R,
    writer: W,
    failed: bool,
}
impl<R: Read, W: Write> Transport<R, W> {
    /// Takes ownership of already-established IPC streams; opens no native resources.
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            failed: false,
        }
    }
    /// Receive one validated packet. Errors are terminal; no stream resynchronization.
    pub fn receive(&mut self) -> io::Result<Message> {
        if self.failed {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        let result = read_message(&mut self.reader);
        self.failed = result.is_err();
        result
    }
    /// Validate, write and flush one packet. Errors poison the transport.
    pub fn send(&mut self, message: &Message) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        let result = write_message(&mut self.writer, message);
        self.failed = result.is_err();
        result
    }
    /// Recover the original streams for explicit backend cleanup.
    pub fn into_inner(self) -> (R, W) {
        (self.reader, self.writer)
    }
}
