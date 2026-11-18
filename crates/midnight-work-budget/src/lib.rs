//! Process-wide fair I/O admission. Requests are admitted in strict FIFO order.
//!
//! Owners consume capacity only while they perform I/O. Call-scoped owners hold a permit
//! for one bounded call. A retained owner, such as a transfer job, keeps one permit
//! identity for its lifetime but holds capacity only while it executes native steps: it
//! releases the permit before waiting for a user decision, a pause or another job, yields
//! it at its next step boundary while a call-scoped request is queued, and readmits it
//! through the same FIFO queue before its next step. Queued exclusive requests and later
//! shared requests therefore wait for at most one step of each retained owner, never for
//! a whole transfer.

#[cfg(feature = "resident-workers")]
pub mod resident;

use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
    time::Duration,
};

pub const DEFAULT_IO_LIMIT: u8 = 4;
pub const MAX_IO_LIMIT: u8 = 32;
const POLL: Duration = Duration::from_millis(10);

pub fn shared_io() -> Arc<Gate> {
    static IO: OnceLock<Arc<Gate>> = OnceLock::new();
    IO.get_or_init(|| Arc::new(Gate::new(DEFAULT_IO_LIMIT)))
        .clone()
}

/// Request semantics survive changes to the configured limit while queued or held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Shared,
    Exclusive,
}

/// Only positive identification of local SSD storage admits concurrent source I/O.
pub fn storage_admission(confirmed_local_ssd: bool) -> Admission {
    if confirmed_local_ssd {
        Admission::Shared
    } else {
        Admission::Exclusive
    }
}

pub struct Gate {
    state: Mutex<State>,
    changed: Condvar,
}
struct State {
    limit: u8,
    active_shared: u8,
    exclusive: bool,
    next: u64,
    queue: VecDeque<Waiter>,
}
/// A queued request. Async waiters keep their waker here; blocking waiters use the condvar.
struct Waiter {
    ticket: u64,
    admission: Admission,
    /// Retained owners wait for whole jobs; they never ask other retained owners to yield.
    retained: bool,
    waker: Option<Waker>,
}
impl State {
    fn eligible(&self, admission: Admission) -> bool {
        !self.exclusive
            && match admission {
                Admission::Shared => self.active_shared < self.limit,
                Admission::Exclusive => self.active_shared == 0,
            }
    }
    fn charge(&mut self, admission: Admission) {
        match admission {
            Admission::Shared => self.active_shared += 1,
            Admission::Exclusive => self.exclusive = true,
        }
    }
    fn refund(&mut self, admission: Admission) {
        match admission {
            Admission::Shared => self.active_shared -= 1,
            Admission::Exclusive => self.exclusive = false,
        }
    }
    fn enqueue(&mut self, admission: Admission, retained: bool) -> u64 {
        let ticket = self.next;
        self.next = self.next.wrapping_add(1);
        self.queue.push_back(Waiter {
            ticket,
            admission,
            retained,
            waker: None,
        });
        ticket
    }
    /// Only the queue head may be admitted, so FIFO order is never overtaken.
    fn admit(&mut self, ticket: u64) -> bool {
        let Some(head) = self.queue.front() else {
            return false;
        };
        if head.ticket != ticket || !self.eligible(head.admission) {
            return false;
        }
        let admission = head.admission;
        self.queue.pop_front();
        self.charge(admission);
        true
    }
    fn withdraw(&mut self, ticket: u64) -> bool {
        let Some(index) = self.queue.iter().position(|waiter| waiter.ticket == ticket) else {
            return false;
        };
        self.queue.remove(index);
        true
    }
}
/// Observational counters for diagnostics and coordination tests; never used to admit work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Remaining shared capacity; zero while exclusive or draining after a decrease.
    pub available: u8,
    pub queued: usize,
    /// May temporarily exceed the current limit while preexisting owners drain.
    pub active_shared: u8,
    pub exclusive: bool,
}
impl Gate {
    pub fn snapshot(&self) -> Snapshot {
        let state = self.lock();
        Snapshot {
            available: if state.exclusive {
                0
            } else {
                state.limit.saturating_sub(state.active_shared)
            },
            queued: state.queue.len(),
            active_shared: state.active_shared,
            exclusive: state.exclusive,
        }
    }
    /// Construct an isolated gate from a trusted internal limit. Validate settings with set_limit.
    pub fn new(limit: u8) -> Self {
        assert!((1..=MAX_IO_LIMIT).contains(&limit));
        Self {
            state: Mutex::new(State {
                limit,
                active_shared: 0,
                exclusive: false,
                next: 0,
                queue: VecDeque::new(),
            }),
            changed: Condvar::new(),
        }
    }
    /// Change future admission without revoking held permits or changing queued request semantics.
    pub fn set_limit(&self, limit: u8) -> Result<(), &'static str> {
        if !(1..=MAX_IO_LIMIT).contains(&limit) {
            return Err("Disk I/O concurrency must be between 1 and 32.");
        }
        let mut state = self.lock();
        state.limit = limit;
        self.notify(state);
        Ok(())
    }
    pub fn effective_limit(&self) -> u8 {
        self.lock().limit
    }
    /// Try immediate call-scoped admission without queuing or overtaking an existing waiter.
    pub fn try_acquire(self: &Arc<Self>, admission: Admission) -> Option<Permit> {
        self.try_admit(admission, false)
    }
    /// Try immediate admission of a retained permit (see the crate documentation).
    pub fn try_acquire_retained(self: &Arc<Self>, admission: Admission) -> Option<Permit> {
        self.try_admit(admission, true)
    }
    /// Blocking call-scoped FIFO admission: an exclusive HDD request is never starved by a
    /// stream of SSD requests. `cancelled` runs without the gate lock; cancellation
    /// withdraws the queued request within one short polling interval, also when
    /// `cancelled` unwinds.
    pub fn acquire(
        self: &Arc<Self>,
        admission: Admission,
        cancelled: impl Fn() -> bool,
    ) -> Option<Permit> {
        if cancelled() {
            return None;
        }
        let permit = Permit::new(self.clone(), admission, false, false);
        let mut queued = Queued {
            gate: self,
            ticket: self.lock().enqueue(admission, false),
            admitted: false,
        };
        loop {
            let mut state = self.lock();
            if state.admit(queued.ticket) {
                queued.admitted = true;
                permit.held.store(true, Ordering::Release);
                self.notify(state);
                return Some(permit);
            }
            let (state, _) = self
                .changed
                .wait_timeout(state, POLL)
                .unwrap_or_else(|error| error.into_inner());
            // Admission is re-checked under the lock before any further wait, so a
            // change while `cancelled` runs unlocked is never missed.
            drop(state);
            if cancelled() {
                return None;
            }
        }
    }
    /// Async FIFO admission of a retained permit. Dropping the future before it completes
    /// withdraws the request; the gate lock is never held across an await. Callers observe
    /// their own cancellation while awaiting it.
    pub fn acquire_retained(self: &Arc<Self>, admission: Admission) -> Acquire {
        Acquire {
            permit: Some(Permit::new(self.clone(), admission, true, false)),
            ticket: None,
        }
    }
    fn try_admit(self: &Arc<Self>, admission: Admission, retained: bool) -> Option<Permit> {
        let mut state = self.lock();
        if !state.queue.is_empty() || !state.eligible(admission) {
            return None;
        }
        state.charge(admission);
        Some(Permit::new(self.clone(), admission, retained, true))
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
    fn withdraw(&self, ticket: u64) {
        let mut state = self.lock();
        if state.withdraw(ticket) {
            self.notify(state);
        }
    }
    /// Publish a state change: blocking waiters re-check, and the async queue head is woken.
    /// Only the head can be admitted; each admission or withdrawal wakes the next head.
    fn notify(&self, mut state: MutexGuard<'_, State>) {
        let waker = state
            .queue
            .front_mut()
            .and_then(|waiter| waiter.waker.take());
        drop(state);
        self.changed.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// Withdraws a blocking request on every exit, including an unwinding `cancelled`.
struct Queued<'a> {
    gate: &'a Gate,
    ticket: u64,
    admitted: bool,
}
impl Drop for Queued<'_> {
    fn drop(&mut self) {
        if !self.admitted {
            self.gate.withdraw(self.ticket);
        }
    }
}

/// Future returned by [`Gate::acquire_retained`].
pub struct Acquire {
    permit: Option<Permit>,
    ticket: Option<u64>,
}
impl Future for Acquire {
    type Output = Permit;
    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Permit> {
        let this = &mut *self;
        let permit = this
            .permit
            .as_ref()
            .expect("I/O admission polled after completion");
        match permit.poll_admit(&mut this.ticket, context) {
            Poll::Ready(()) => Poll::Ready(
                this.permit
                    .take()
                    .expect("I/O admission polled after completion"),
            ),
            Poll::Pending => Poll::Pending,
        }
    }
}
impl Drop for Acquire {
    fn drop(&mut self) {
        if let (Some(ticket), Some(permit)) = (self.ticket.take(), self.permit.as_ref()) {
            permit.gate.withdraw(ticket);
        }
    }
}

/// Future returned by [`Permit::reacquire`].
pub struct Reacquire<'a> {
    permit: &'a Permit,
    ticket: Option<u64>,
}
impl Future for Reacquire<'_> {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        this.permit.poll_admit(&mut this.ticket, context)
    }
}
impl Drop for Reacquire<'_> {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            self.permit.gate.withdraw(ticket);
        }
    }
}

/// One admission to an exact gate. It consumes capacity while held: until it is dropped
/// (after the admitted call actually returns, including after cancellation or a limit
/// decrease) or explicitly released. A released permit keeps its identity and can be
/// readmitted through the FIFO queue.
pub struct Permit {
    gate: Arc<Gate>,
    admission: Admission,
    retained: bool,
    /// Changed only under the gate lock, so it always matches the gate's counters.
    held: AtomicBool,
}
impl Permit {
    fn new(gate: Arc<Gate>, admission: Admission, retained: bool, held: bool) -> Self {
        Self {
            gate,
            admission,
            retained,
            held: AtomicBool::new(held),
        }
    }
    /// Whether this guard belongs to the exact configured gate, held or released.
    pub fn belongs_to(&self, gate: &Arc<Gate>) -> bool {
        Arc::ptr_eq(&self.gate, gate)
    }
    pub fn is_shared(&self) -> bool {
        self.admission == Admission::Shared
    }
    /// Whether this guard currently consumes capacity. Only a held permit admits I/O.
    pub fn is_held(&self) -> bool {
        self.held.load(Ordering::Acquire)
    }
    /// Whether a call-scoped request is queued. A retained owner then releases this permit
    /// at its next step boundary and readmits it, so that request waits for at most one
    /// step instead of the owner's whole job. Queued retained owners never count.
    pub fn is_contended(&self) -> bool {
        self.gate.lock().queue.iter().any(|waiter| !waiter.retained)
    }
    /// Return the capacity now, e.g. before waiting for a user decision. Idempotent;
    /// returns whether capacity was returned by this call. Clones of an `Arc<Permit>`
    /// observe the release, so forgotten clones cannot retain capacity.
    pub fn release(&self) -> bool {
        let mut state = self.gate.lock();
        if !self.held.swap(false, Ordering::AcqRel) {
            return false;
        }
        state.refund(self.admission);
        self.gate.notify(state);
        true
    }
    /// Readmit a released permit at once when nothing is queued and capacity allows;
    /// like `Gate::try_acquire`, this never overtakes a waiter. Returns whether held.
    pub fn try_reacquire(&self) -> bool {
        let mut state = self.gate.lock();
        if self.held.load(Ordering::Acquire) {
            return true;
        }
        if !state.queue.is_empty() || !state.eligible(self.admission) {
            return false;
        }
        state.charge(self.admission);
        self.held.store(true, Ordering::Release);
        true
    }
    /// Readmit a released permit through the FIFO queue; completes at once while held.
    /// Dropping the future before completion withdraws the request.
    pub fn reacquire(&self) -> Reacquire<'_> {
        Reacquire {
            permit: self,
            ticket: None,
        }
    }
    /// Charge a released shared permit immediately, without queuing, for bounded
    /// teardown that must not wait for capacity (cancellation cleanup). This may
    /// briefly exceed the configured limit or overlap an exclusive owner. An exclusive
    /// permit is reclaimed only when that is possible at once; returns whether held.
    pub fn reclaim(&self) -> bool {
        let mut state = self.gate.lock();
        if self.held.load(Ordering::Acquire) {
            return true;
        }
        match self.admission {
            Admission::Shared => state.active_shared += 1,
            Admission::Exclusive if !state.exclusive && state.active_shared == 0 => {
                state.exclusive = true;
            }
            Admission::Exclusive => return false,
        }
        self.held.store(true, Ordering::Release);
        true
    }
    fn poll_admit(&self, ticket: &mut Option<u64>, context: &mut Context<'_>) -> Poll<()> {
        let mut state = self.gate.lock();
        if self.held.load(Ordering::Acquire) {
            // Held already (never released, or reclaimed): withdraw any queued request.
            if let Some(queued) = ticket.take()
                && state.withdraw(queued)
            {
                self.gate.notify(state);
            }
            return Poll::Ready(());
        }
        let queued = match *ticket {
            Some(queued) if state.queue.iter().any(|waiter| waiter.ticket == queued) => queued,
            _ => *ticket.insert(state.enqueue(self.admission, self.retained)),
        };
        if state.admit(queued) {
            *ticket = None;
            self.held.store(true, Ordering::Release);
            self.gate.notify(state);
            return Poll::Ready(());
        }
        if let Some(waiter) = state
            .queue
            .iter_mut()
            .find(|waiter| waiter.ticket == queued)
            && waiter
                .waker
                .as_ref()
                .is_none_or(|waker| !waker.will_wake(context.waker()))
        {
            waiter.waker = Some(context.waker().clone());
        }
        Poll::Pending
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
