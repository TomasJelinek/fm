//! Windows Unicode clipboard access, owned and serialized by the backend worker.
use crate::file_drag_drop::wait::Wake;
use midnight_core::{ClipboardService, ClipboardSession, ClipboardWaker, FileClipboard, FsError};
use std::{io, path::Path, ptr, time::Duration};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;

#[derive(Default)]
pub struct NativeClipboard;

struct Session {
    text: arboard::Clipboard,
    files: Option<crate::file_clipboard::Files>,
    /// Ends `wait_pending`; its wakers stay harmless after the session was dropped.
    wake: Wake,
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
        let text = arboard::Clipboard::new().map_err(failure)?;
        Ok(Box::new(Session {
            text,
            files: None,
            wake: Wake::new().map_err(os_failure)?,
        }))
    }
}

impl ClipboardSession for Session {
    fn read_text(&mut self) -> Result<String, FsError> {
        self.files()?.read_text()
    }
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
            files.service_pending();
        }
        Ok(())
    }
    fn owns_clipboard(&self) -> bool {
        // Text goes through arboard without an owner window and needs no servicing; only
        // the file clipboard window takes ownership, until it receives WM_DESTROYCLIPBOARD.
        self.files
            .as_ref()
            .is_some_and(|files| files.owns_clipboard())
    }
    fn wait_pending(&mut self, timeout: Duration) -> Result<(), FsError> {
        // The owner thread's input includes a sent WM_DESTROYCLIPBOARD (another owner
        // emptied the clipboard).
        self.wake.wait(timeout).map_err(os_failure)?;
        dispatch_pending();
        Ok(())
    }
    fn waker(&self) -> Option<ClipboardWaker> {
        Some(self.wake.waker())
    }
}

/// Messages dispatched per wait; the same per-tick cap as `Files::service_pending`.
const MESSAGES_PER_WAIT: usize = 128;

/// Dispatch at most `MESSAGES_PER_WAIT` queued messages of the owner thread; each
/// PeekMessageW first delivers pending sent messages such as WM_DESTROYCLIPBOARD. The
/// drain is not filtered by window: the owner thread hosts only this session's window,
/// and a message left queued would end every later MWMO_INPUTAVAILABLE wait at once.
fn dispatch_pending() {
    let mut message = MSG::default();
    for _ in 0..MESSAGES_PER_WAIT {
        // SAFETY: retrieves one message of the calling thread into a local MSG.
        if unsafe { PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) } == 0 {
            break;
        }
        // SAFETY: the message was just retrieved on this thread; dispatching calls the
        // procedure of its target window, which this thread owns (none for a thread message).
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn failure(error: arboard::Error) -> FsError {
    os_failure(io::Error::other(error.to_string()))
}

fn os_failure(error: io::Error) -> FsError {
    FsError::from_io("Windows clipboard", Path::new(""), error)
}
