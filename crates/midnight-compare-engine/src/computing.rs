//! How many comparisons prepare their content at once.
//!
//! Any number of comparisons may be open; each keeps a resident worker that mostly waits
//! for input. Preparing a comparison (reading and aligning text, starting and decoding in
//! the image workers) is the CPU- and memory-heavy phase, so the service admits at most
//! `COMPUTING` preparations at once, first come first served. The others wait,
//! cancellable, with the status `WAITING`, and show `PREPARING` once admitted. Opening the
//! files and resolving the text font come before the wait: an open that blocks on an
//! unresponsive share holds no permit. A replacement queued while a comparison waits ends
//! its wait.
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
pub(crate) const PREPARING: &str = "Preparing the comparison…";

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
    /// Fails once `cancel` is set, and gives up with `None` once `superseded` is true (a
    /// replacement comparison was queued); `interrupt` wakes waiters to observe either.
    /// `superseded` runs with the gate locked.
    pub fn acquire(
        self: &Arc<Self>,
        cancel: &AtomicBool,
        superseded: impl Fn() -> bool,
        waiting: impl FnOnce(),
    ) -> Result<Option<Permit>, String> {
        let mut state = self.lock();
        check_cancel(cancel)?;
        if state.free > 0 && state.queue.is_empty() {
            state.free -= 1;
            return Ok(Some(Permit(self.clone())));
        }
        let ticket = Ticket {
            gate: self,
            id: state.next_ticket,
        };
        state.next_ticket = ticket.id.wrapping_add(1);
        state.queue.push_back(ticket.id);
        drop(state);
        // The ticket leaves the queue however this ends, also when `waiting` unwinds:
        // a ticket left at the front would hold every later comparison back.
        waiting();
        let mut state = self.lock();
        loop {
            check_cancel(cancel)?;
            if superseded() {
                return Ok(None);
            }
            if state.free > 0 && state.queue.front() == Some(&ticket.id) {
                state.free -= 1;
                // `ticket` leaves the queue once `state` is released, and wakes the next
                // ticket: another permit may be free for it.
                return Ok(Some(Permit(self.clone())));
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    /// Wake waiters to observe a cancellation or replacement made before this call.
    pub fn interrupt(&self) {
        let _state = self.lock();
        self.changed.notify_all();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

/// A place in the queue; dropping it leaves the queue and wakes the next waiter.
struct Ticket<'a> {
    gate: &'a Gate,
    id: u64,
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        self.gate.lock().queue.retain(|waiting| *waiting != self.id);
        // The next ticket may now be first in line, or a permit may be free for it.
        self.gate.changed.notify_all();
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.lock().free += 1;
        self.0.changed.notify_all();
    }
}
