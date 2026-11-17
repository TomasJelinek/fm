//! Windows Unicode clipboard access, owned and serialized by the backend worker.
use midnight_core::{ClipboardService, ClipboardSession, ClipboardWaker, FileClipboard, FsError};
use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::Path,
    ptr,
    sync::Arc,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::WAIT_FAILED,
    System::Threading::{CreateEventW, SetEvent},
    UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE,
        PeekMessageW, QS_ALLINPUT, TranslateMessage,
    },
};

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;

#[derive(Default)]
pub struct NativeClipboard;

struct Session {
    text: arboard::Clipboard,
    files: Option<crate::file_clipboard::Files>,
    /// Auto-reset event that ends `wait_pending`. Each waker holds a clone, so the handle
    /// stays open, and a late wake harmless, until the last holder is dropped.
    wake: Arc<OwnedHandle>,
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
            wake: Arc::new(wake_event()?),
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
        let event = self.wake.as_raw_handle();
        // SAFETY: waits on one live event handle owned by this session and on the message
        // queue of the calling owner thread. MWMO_INPUTAVAILABLE also returns for input that
        // an earlier PeekMessageW saw but left queued; a sent WM_DESTROYCLIPBOARD (another
        // owner emptied the clipboard) is input too.
        let result = unsafe {
            MsgWaitForMultipleObjectsEx(
                1,
                &event,
                wait_milliseconds(timeout),
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            )
        };
        if result == WAIT_FAILED {
            return Err(os_failure(io::Error::last_os_error()));
        }
        dispatch_pending();
        Ok(())
    }
    fn waker(&self) -> Option<ClipboardWaker> {
        let event = self.wake.clone();
        Some(Arc::new(move || {
            // SAFETY: the captured Arc keeps this event handle open while the waker exists;
            // SetEvent neither blocks nor waits for the owner thread.
            unsafe { SetEvent(event.as_raw_handle()) };
        }))
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

/// An unnamed auto-reset event: a wake made while no wait runs keeps it signaled until
/// the next wait consumes it, so no wake is lost and several wakes end one wait.
fn wake_event() -> Result<OwnedHandle, FsError> {
    // SAFETY: default security, auto-reset, initially nonsignaled and unnamed.
    let event = unsafe { CreateEventW(ptr::null(), 0, 0, ptr::null()) };
    if event.is_null() {
        return Err(os_failure(io::Error::last_os_error()));
    }
    // SAFETY: ownership starts only after successful native handle creation.
    Ok(unsafe { OwnedHandle::from_raw_handle(event) })
}

/// `timeout` in whole milliseconds for a Win32 wait: rounded up, so that a short nonzero
/// timeout never becomes a zero-time poll, and kept finite (below INFINITE).
fn wait_milliseconds(timeout: Duration) -> u32 {
    let finite = u32::MAX - 1;
    u32::try_from(timeout.as_nanos().div_ceil(1_000_000)).map_or(finite, |ms| ms.min(finite))
}

fn failure(error: arboard::Error) -> FsError {
    os_failure(io::Error::other(error.to_string()))
}

fn os_failure(error: io::Error) -> FsError {
    FsError::from_io("Windows clipboard", Path::new(""), error)
}
