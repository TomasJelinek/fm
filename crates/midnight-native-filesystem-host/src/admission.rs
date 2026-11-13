//! Preserve independent legacy cleanup and lazily reserved Rename control capacity.
use crate::{Cancel, client::check_cancel};
use midnight_filesystem_plugin_api::Failure;
use std::sync::{Condvar, Mutex, MutexGuard};

// SDK instances permit eight calls. The host holds each admission until its
// reply buffer is released, bounding buffers/reservations as well as callbacks.
const ORDINARY_CALLS: usize = 7;
const RENAME_ORDINARY_CALLS: usize = 6;

#[derive(Debug, Default)]
struct Active {
    ordinary: usize,
    rename_reserved: bool,
    control: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Admission {
    active: Mutex<Active>,
    available: Condvar,
    cleanup: Mutex<()>,
}
impl Admission {
    pub(crate) fn enter(&self, cancelled: Cancel<'_>) -> Result<Ordinary<'_>, Failure> {
        self.enter_general(cancelled, false)
    }
    pub(crate) fn enter_rename(&self, cancelled: Cancel<'_>) -> Result<Ordinary<'_>, Failure> {
        self.enter_general(cancelled, true)
    }
    fn enter_general(
        &self,
        cancelled: Cancel<'_>,
        reserve_rename: bool,
    ) -> Result<Ordinary<'_>, Failure> {
        let mut active = lock(&self.active);
        check_cancel(cancelled)?;
        if reserve_rename {
            // Persist before waiting: fresh Browse work cannot refill slot seven.
            // A waiter canceled later owns no permit and cannot undo this partition.
            active.rename_reserved = true;
        }
        loop {
            check_cancel(cancelled)?;
            let limit = if active.rename_reserved {
                RENAME_ORDINARY_CALLS
            } else {
                ORDINARY_CALLS
            };
            if active.ordinary < limit {
                active.ordinary += 1;
                return Ok(Ordinary(self));
            }
            active = self
                .available
                .wait_timeout(active, std::time::Duration::from_millis(10))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }
    pub(crate) fn control(&self) -> Control<'_> {
        let mut active = lock(&self.active);
        active.rename_reserved = true;
        while active.ordinary > RENAME_ORDINARY_CALLS || active.control {
            // This is independent of user cancellation. Seven existing ordinary
            // owners must actually release one before control plus cleanup can fit.
            active = self
                .available
                .wait(active)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        active.control = true;
        Control(self)
    }
    pub(crate) fn cleanup(&self) -> MutexGuard<'_, ()> {
        lock(&self.cleanup)
    }
}
pub(crate) struct Ordinary<'a>(&'a Admission);
impl Drop for Ordinary<'_> {
    fn drop(&mut self) {
        lock(&self.0.active).ordinary -= 1;
        // Control and general waiters have different predicates on the same state.
        self.0.available.notify_all();
    }
}
pub(crate) struct Control<'a>(&'a Admission);
impl Drop for Control<'_> {
    fn drop(&mut self) {
        lock(&self.0.active).control = false;
        self.0.available.notify_all();
    }
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };

    #[test]
    fn slow_reads_leave_cleanup_capacity_and_cancelled_waiters_hold_no_permit() {
        let admission = Arc::new(Admission::default());
        let held: Vec<_> = (0..7)
            .map(|_| admission.enter(&|| false).unwrap())
            .collect();
        let (send, received) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_admission = admission.clone();
        let worker_cancel = cancel.clone();
        let worker = std::thread::spawn(move || {
            let result = worker_admission.enter(&|| worker_cancel.load(Ordering::Acquire));
            send.send(result.is_ok()).unwrap();
        });
        // An eighth normal read may not consume the destructor's reserved slot.
        assert!(received.recv_timeout(Duration::from_millis(80)).is_err());
        {
            let _cleanup = admission.cleanup();
            assert_eq!(lock(&admission.active).ordinary, 7);
        }
        cancel.store(true, Ordering::Release);
        assert!(!received.recv_timeout(Duration::from_secs(2)).unwrap());
        worker.join().unwrap();
        drop(held);
        assert_eq!(lock(&admission.active).ordinary, 0);
        let _again = admission.enter(&|| false).unwrap();
    }
}
