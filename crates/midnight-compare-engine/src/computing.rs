//! How many comparisons prepare their content at once.
//!
//! Any number of comparisons may be open; each keeps a resident worker that mostly waits
//! for input. Preparing a comparison (opening both files, reading and aligning text,
//! starting and decoding in the image workers) is the CPU- and memory-heavy phase, so
//! the service admits at most `COMPUTING` preparations at once, first come first served.
//! The others wait, cancellable, with the status `WAITING`.
use crate::check_cancel;
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, atomic::AtomicBool},
};

/// Two preparations at once, as the earlier limit of two open comparisons allowed: the
/// peak memory of concurrent preparation stays what it was (per comparison at most about
/// 2.5 times its two texts, or two image workers of at most 1 GiB each).
pub(crate) const COMPUTING: usize = 2;
pub(crate) const WAITING: &str = "Waiting for other comparisons to finish…";

pub(crate) struct Gate {
    state: Mutex<State>,
    changed: Condvar,
}

struct State {
    free: usize,
    next_ticket: u64,
    /// Waiting tickets in arrival order; only the first may take a free permit.
    queue: VecDeque<u64>,
}

/// One admitted preparation. Dropping it admits the next waiting one.
pub(crate) struct Permit(Arc<Gate>);

impl Gate {
    pub fn new(permits: usize) -> Self {
        Self {
            state: Mutex::new(State {
                free: permits,
                next_ticket: 0,
                queue: VecDeque::new(),
            }),
            changed: Condvar::new(),
        }
    }

    /// Wait for a permit. `waiting` runs once, before waiting, when none is free at once.
    /// Fails once `cancel` is set; `interrupt` wakes waiters to observe it.
    pub fn acquire(
        self: &Arc<Self>,
        cancel: &AtomicBool,
        waiting: impl FnOnce(),
    ) -> Result<Permit, String> {
        let mut state = self.lock();
        check_cancel(cancel)?;
        if state.free > 0 && state.queue.is_empty() {
            state.free -= 1;
            return Ok(Permit(self.clone()));
        }
        let ticket = state.next_ticket;
        state.next_ticket = ticket.wrapping_add(1);
        state.queue.push_back(ticket);
        drop(state);
        waiting();
        let mut state = self.lock();
        loop {
            if let Err(error) = check_cancel(cancel) {
                state.queue.retain(|waiting| *waiting != ticket);
                // The next ticket may now be first in line.
                self.changed.notify_all();
                return Err(error);
            }
            if state.free > 0 && state.queue.front() == Some(&ticket) {
                state.queue.pop_front();
                state.free -= 1;
                // Another permit may be free for the next ticket.
                self.changed.notify_all();
                return Ok(Permit(self.clone()));
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    /// Wake waiters to observe a cancellation set before this call.
    pub fn interrupt(&self) {
        let _state = self.lock();
        self.changed.notify_all();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.lock().free += 1;
        self.0.changed.notify_all();
    }
}
