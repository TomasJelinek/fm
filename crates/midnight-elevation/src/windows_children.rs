//! At most two UAC requests/children, including live authorization sessions and cancelled or
//! closed helpers that have not exited yet.
use std::{
    io,
    os::windows::io::{AsRawHandle, OwnedHandle},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::Threading::{TerminateProcess, WaitForSingleObject},
};

/// How long a request waits for closed helpers to exit when they fill the budget: a session
/// that ended just before (its pane left the folder or retries it) frees its slot this way.
const RETIRING_WAIT: Duration = Duration::from_secs(2);
const RETIRING_POLL: Duration = Duration::from_millis(20);

static CHILDREN: Mutex<Children> = Mutex::new(Children {
    reserved: 0,
    retiring: Vec::new(),
});
struct Children {
    reserved: usize,
    retiring: Vec<OwnedHandle>,
}
pub(super) struct Reservation {
    process: Option<OwnedHandle>,
}
impl Reservation {
    pub(super) fn acquire() -> io::Result<Self> {
        let deadline = Instant::now() + RETIRING_WAIT;
        loop {
            let mut children = CHILDREN.lock().unwrap_or_else(|error| error.into_inner());
            // A retained process handle cannot be reused for an unrelated PID.
            children.retiring.retain(|process| unsafe {
                WaitForSingleObject(process.as_raw_handle(), 0) != WAIT_OBJECT_0
            });
            if children.reserved + children.retiring.len() < 2 {
                children.reserved += 1;
                return Ok(Self { process: None });
            }
            if children.retiring.is_empty() || Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Two authorizations are still active. Finish or cancel the system's \
                     authorization prompt, or leave a folder listed with authorization, first.",
                ));
            }
            drop(children);
            thread::sleep(RETIRING_POLL);
        }
    }
    pub(super) fn launched(&mut self, process: OwnedHandle) -> io::Result<OwnedHandle> {
        self.process = Some(process);
        self.process.as_ref().expect("just retained").try_clone()
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut children = CHILDREN.lock().unwrap_or_else(|error| error.into_inner());
        children.reserved -= 1;
        if let Some(process) = self.process.take() {
            // Only the dedicated, launched read-only helper can be terminated here. Windows may
            // refuse termination across an elevation boundary; a refusal never frees its slot.
            // Channel closure and the helper's parent/deadline monitor independently cancel it.
            if unsafe { WaitForSingleObject(process.as_raw_handle(), 0) } != WAIT_OBJECT_0 {
                unsafe {
                    TerminateProcess(process.as_raw_handle(), 1);
                }
            }
            if unsafe { WaitForSingleObject(process.as_raw_handle(), 0) } != WAIT_OBJECT_0 {
                children.retiring.push(process);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authorization_reservations_are_bounded_even_before_process_launch() {
        let first = Reservation::acquire().unwrap();
        let second = Reservation::acquire().unwrap();
        assert!(Reservation::acquire().is_err());
        drop(first);
        let replacement = Reservation::acquire().unwrap();
        drop(second);
        drop(replacement);
    }
}
