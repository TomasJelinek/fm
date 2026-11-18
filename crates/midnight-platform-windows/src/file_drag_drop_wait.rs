//! The idle wait of a session's owner thread, shared by the drag/drop and clipboard
//! sessions: the thread's message queue and the session's wake event.
use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
    sync::Arc,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::WAIT_FAILED,
    System::Threading::{CreateEventW, SetEvent},
    UI::WindowsAndMessaging::{MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, QS_ALLINPUT},
};

/// A session's wake event: an unnamed auto-reset event. A wake made while no wait runs
/// keeps it signaled until the next wait consumes it, so no wake is lost and several
/// wakes end one wait. Each waker holds the handle, so it stays open, and a late wake
/// harmless, until the last holder is dropped.
pub(crate) struct Wake(Arc<OwnedHandle>);

impl Wake {
    pub(crate) fn new() -> io::Result<Self> {
        // SAFETY: default security, auto-reset, initially nonsignaled and unnamed.
        let event = unsafe { CreateEventW(ptr::null(), 0, 0, ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: ownership starts only after successful native handle creation.
        let event = unsafe { OwnedHandle::from_raw_handle(event) };
        Ok(Self(Arc::new(event)))
    }

    /// Ends the current or the next `wait`. Any thread may call it; it never blocks.
    pub(crate) fn waker(&self) -> Arc<dyn Fn() + Send + Sync> {
        let event = self.0.clone();
        Arc::new(move || {
            // SAFETY: the captured Arc keeps this event handle open while the waker exists;
            // SetEvent neither blocks nor waits for the waiting thread.
            unsafe { SetEvent(event.as_raw_handle()) };
        })
    }

    /// Block the calling thread until this event is signaled (the wait consumes it), input
    /// arrives in the thread's message queue or `timeout` elapses. Sent messages, such as
    /// a call that OLE marshals into the apartment or WM_DESTROYCLIPBOARD, are input too,
    /// and MWMO_INPUTAVAILABLE also returns for messages that an earlier PeekMessageW saw
    /// but left queued. The caller dispatches that input.
    pub(crate) fn wait(&self, timeout: Duration) -> io::Result<()> {
        let event = self.0.as_raw_handle();
        // SAFETY: waits on one live event handle, owned by this wake, and on the message
        // queue of the calling thread.
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
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// `timeout` in whole milliseconds for a Win32 wait: rounded up, so that a short nonzero
/// timeout never becomes a zero-time poll, and kept finite (below INFINITE).
fn wait_milliseconds(timeout: Duration) -> u32 {
    let finite = u32::MAX - 1;
    u32::try_from(timeout.as_nanos().div_ceil(1_000_000)).map_or(finite, |ms| ms.min(finite))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use windows_sys::Win32::{Foundation::WAIT_TIMEOUT, System::Threading::WaitForSingleObject};

    /// Elapses only when the wake that should end a wait does not.
    const FAILURE_WAIT: Duration = Duration::from_secs(30);

    #[test]
    fn wakes_coalesce_into_one_wait_and_outlive_their_owner() {
        let wake = Wake::new().unwrap();
        let waker = wake.waker();
        // Wakes made before a wait end it, and together they end only that wait.
        waker();
        waker();
        let started = Instant::now();
        wake.wait(FAILURE_WAIT).unwrap();
        assert!(
            started.elapsed() < FAILURE_WAIT / 2,
            "an earlier wake must end the next wait"
        );
        assert_eq!(
            // SAFETY: polls one live event handle without waiting.
            unsafe { WaitForSingleObject(wake.0.as_raw_handle(), 0) },
            WAIT_TIMEOUT,
            "one wait consumes coalesced wakes"
        );
        // Every waker keeps the event open: a call after the owner was dropped signals
        // an event that nobody waits on, and the last waker closes it.
        let event = Arc::downgrade(&wake.0);
        drop(wake);
        assert!(event.upgrade().is_some(), "a waker keeps the event open");
        waker();
        drop(waker);
        assert!(event.upgrade().is_none(), "the last waker closes the event");
    }

    #[test]
    fn wait_timeouts_round_up_and_stay_finite() {
        assert_eq!(wait_milliseconds(Duration::ZERO), 0);
        assert_eq!(wait_milliseconds(Duration::from_nanos(1)), 1);
        assert_eq!(wait_milliseconds(Duration::from_millis(10)), 10);
        assert_eq!(wait_milliseconds(Duration::from_micros(10_001)), 11);
        assert_eq!(wait_milliseconds(Duration::MAX), u32::MAX - 1);
    }
}
