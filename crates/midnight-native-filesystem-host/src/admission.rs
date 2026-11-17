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
    #[cfg(test)]
    probe: probe::Probe,
}

/// Test-only observation that a caller is blocked on the latest admission state.
/// Only releases can admit a parked caller, and each release advances the epoch, so
/// a caller parked at the current epoch provably stays blocked until the next release.
#[cfg(test)]
mod probe {
    use std::{collections::HashMap, thread::ThreadId};
    #[derive(Debug, Default)]
    pub(super) struct Probe {
        epoch: u64,
        parked: HashMap<ThreadId, u64>,
    }
    impl Probe {
        pub(super) fn park(&mut self) {
            self.parked.insert(std::thread::current().id(), self.epoch);
        }
        pub(super) fn wake(&mut self) {
            self.parked.remove(&std::thread::current().id());
        }
        pub(super) fn released(&mut self) {
            self.epoch += 1;
        }
        pub(super) fn parked_on_current_state(&self, thread: ThreadId) -> bool {
            self.parked.get(&thread) == Some(&self.epoch)
        }
    }
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
            #[cfg(test)]
            active.probe.wake();
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
            #[cfg(test)]
            active.probe.park();
            // Cancellation has no wake signal; the timeout only rechecks it.
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
            #[cfg(test)]
            active.probe.park();
            // This is independent of user cancellation. Seven existing ordinary
            // owners must actually release one before control plus cleanup can fit.
            active = self
                .available
                .wait(active)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            #[cfg(test)]
            active.probe.wake();
        }
        active.control = true;
        Control(self)
    }
    pub(crate) fn cleanup(&self) -> MutexGuard<'_, ()> {
        lock(&self.cleanup)
    }
    #[cfg(test)]
    pub(crate) fn parked_on_current_state(&self, thread: std::thread::ThreadId) -> bool {
        lock(&self.active).probe.parked_on_current_state(thread)
    }
}
pub(crate) struct Ordinary<'a>(&'a Admission);
impl Drop for Ordinary<'_> {
    fn drop(&mut self) {
        let mut active = lock(&self.0.active);
        active.ordinary -= 1;
        #[cfg(test)]
        active.probe.released();
        drop(active);
        // Control and general waiters have different predicates on the same state.
        self.0.available.notify_all();
    }
}
pub(crate) struct Control<'a>(&'a Admission);
impl Drop for Control<'_> {
    fn drop(&mut self) {
        let mut active = lock(&self.0.active);
        active.control = false;
        #[cfg(test)]
        active.probe.released();
        drop(active);
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
        // An eighth normal read may not consume the destructor's reserved slot: it
        // parks on the current state, which only a release (none happens) can change.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !admission.parked_on_current_state(worker.thread().id()) {
            assert!(received.try_recv().is_err(), "an eighth read was admitted");
            assert!(
                std::time::Instant::now() < deadline,
                "the read never waited"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(received.try_recv().is_err());
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
