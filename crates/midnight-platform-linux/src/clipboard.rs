//! Linux clipboard sessions: text through arboard, file lists through this session's
//! X11 owner window. An idle session waits for owner events or its waker in `poll()`.
use midnight_core::{ClipboardService, ClipboardSession, ClipboardWaker, FileClipboard, FsError};
use rustix::{
    event::{EventfdFlags, PollFd, PollFlags, Timespec, eventfd, poll},
    fd::{BorrowedFd, OwnedFd},
    io::Errno,
};
use std::{io, path::Path, sync::Arc, time::Duration};

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;

#[derive(Default)]
pub struct NativeClipboard;

struct Session {
    text: Option<arboard::Clipboard>,
    files: Option<crate::file_clipboard::Files>,
    /// Text that arboard published. arboard serves it from its own connection and
    /// thread, so this session never sees the SelectionClear that ends it; and dropping
    /// the process's last arboard handle stops serving it (after offering it to a
    /// clipboard manager). The session therefore counts it as owned until its own X11
    /// owner replaces it with a file list.
    text_owned: bool,
    /// Ends `wait_pending`; None when no eventfd could be created, and the session then
    /// offers no waker.
    wake: Option<Wake>,
}

impl Session {
    fn new() -> Self {
        Self {
            text: None,
            files: None,
            text_owned: false,
            wake: Wake::new().ok(),
        }
    }

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
        Ok(Box::new(Session::new()))
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
            .map_err(failure)?;
        self.text_owned = true;
        Ok(())
    }
    fn copy_files(&mut self, files: &FileClipboard) -> Result<(), FsError> {
        self.files()?.copy(files)?;
        // The session's X11 window now owns CLIPBOARD; arboard's text is no longer offered.
        self.text_owned = false;
        Ok(())
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
    fn owns_clipboard(&self) -> bool {
        // The session takes only CLIPBOARD, never PRIMARY: through arboard (text) or its
        // X11 owner window (file lists), which a SelectionClear from a newer owner ends.
        self.text_owned
            || self
                .files
                .as_ref()
                .is_some_and(crate::file_clipboard::Files::owns_clipboard)
    }
    fn wait_pending(&mut self, timeout: Duration) -> Result<(), FsError> {
        match &mut self.files {
            Some(files) => files.wait(self.wake.as_ref(), timeout),
            // No X11 connection, so no owner events: only a wake or the timeout ends it.
            None => wait(self.wake.as_ref(), None, timeout).map(drop),
        }
    }
    fn waker(&self) -> Option<ClipboardWaker> {
        self.wake.as_ref().map(Wake::waker)
    }
}

/// A session's wake event: a nonblocking eventfd whose counter each waker increments
/// and a wait resets. A wake made while no wait runs keeps it readable until the next
/// wait consumes it, so no wake is lost and several wakes end one wait. Each waker holds
/// the descriptor, so a wake after the session was dropped writes to a still-open
/// eventfd that nobody reads.
#[derive(Clone)]
pub(crate) struct Wake(Arc<OwnedFd>);

impl Wake {
    pub(crate) fn new() -> io::Result<Self> {
        let event = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        Ok(Self(Arc::new(event)))
    }

    pub(crate) fn waker(&self) -> ClipboardWaker {
        let event = self.0.clone();
        Arc::new(move || {
            // Never blocks: the write fails with EAGAIN only when the counter is already
            // near its limit, which leaves the eventfd readable as well.
            let _ = rustix::io::write(&*event, &1u64.to_ne_bytes());
        })
    }

    /// Reset the counter: the wakes made so far end only the current wait.
    fn consume(&self) {
        let mut counter = [0u8; 8];
        // EAGAIN (nothing to consume) needs no handling.
        let _ = rustix::io::read(&*self.0, &mut counter[..]);
    }
}

/// Block until `wake` is signaled, `connection` becomes readable (or reports an error or
/// hang-up) or `timeout` elapses; a signaled wake is consumed. Returns whether the
/// connection may have input, which also holds after an interrupted wait. Without any
/// descriptor this is a bounded park, as the trait's default wait.
pub(crate) fn wait(
    wake: Option<&Wake>,
    connection: Option<BorrowedFd<'_>>,
    timeout: Duration,
) -> Result<bool, FsError> {
    let mut descriptors = Vec::with_capacity(2);
    if let Some(wake) = wake {
        descriptors.push(PollFd::new(&*wake.0, PollFlags::IN));
    }
    if let Some(connection) = connection {
        descriptors.push(PollFd::from_borrowed_fd(connection, PollFlags::IN));
    }
    if descriptors.is_empty() {
        std::thread::park_timeout(timeout);
        return Ok(false);
    }
    match poll(&mut descriptors, Some(&timespec(timeout))) {
        Ok(_) => {}
        // A signal ended the wait early, which the trait allows.
        Err(Errno::INTR) => return Ok(connection.is_some()),
        Err(error) => return Err(os_failure(error.into())),
    }
    // In push order: the wake event first, then the connection.
    let mut ready = descriptors
        .iter()
        .map(|descriptor| !descriptor.revents().is_empty());
    let woken = wake.is_some() && ready.next() == Some(true);
    let readable = connection.is_some() && ready.next() == Some(true);
    if woken && let Some(wake) = wake {
        wake.consume();
    }
    Ok(readable)
}

/// `timeout` as a `ppoll` timespec; a duration beyond its range waits the longest it can.
fn timespec(timeout: Duration) -> Timespec {
    Timespec::try_from(timeout).unwrap_or(Timespec {
        tv_sec: i64::MAX,
        tv_nsec: 0,
    })
}

fn failure(error: arboard::Error) -> FsError {
    os_failure(io::Error::other(error.to_string()))
}

fn os_failure(error: io::Error) -> FsError {
    FsError::from_io("Linux clipboard", Path::new(""), error)
}
