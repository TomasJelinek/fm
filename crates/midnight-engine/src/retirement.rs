//! Bounded cleanup independent of executor and runtime lifetime.
//!
//! Reserve before accepting a payload. A lease occupies one preallocated slot until
//! its payload is actually destroyed; dropping the last domain handle never joins
//! the worker. The integrator derives capacity from its admission/actor ledger.
use std::{
    collections::VecDeque,
    fmt,
    future::Future,
    io,
    ops::{Deref, DerefMut},
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{Arc, Condvar, Mutex, OnceLock},
    task::{Context, Poll, Waker},
    thread::{self, ThreadId},
};

type Payload = Box<dyn Send + 'static>;
type Worker = Box<dyn FnOnce() + Send + 'static>;

/// Creating/reserving this domain takes no provider-bearing payload.
#[derive(Clone)]
pub(crate) struct RetirementDomain {
    control: Arc<Control>,
}

struct Control {
    shared: Arc<Shared>,
}
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
    worker: OnceLock<ThreadId>,
}
struct Queue {
    free: Vec<usize>,
    slots: Vec<Option<Job>>,
    pending: VecDeque<usize>,
    closing: bool,
    free_waiters: Vec<usize>,
    waiters: Vec<Option<Waker>>,
}
struct Job {
    payload: Payload,
    completion: Option<Arc<Completion>>,
}

/// An already-reserved slot. Dropping an unused lease returns just its slot.
pub(crate) struct RetirementLease {
    control: Arc<Control>,
    slot: Option<usize>,
}

/// A startup-reserved registration: at most one acquisition may borrow it at once.
pub(crate) struct RetirementWaiter {
    control: Arc<Control>,
    slot: usize,
}
struct Reservation<'a> {
    waiter: &'a mut RetirementWaiter,
    completed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RetirementBusy;
impl fmt::Display for RetirementBusy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Retirement capacity is occupied")
    }
}
impl std::error::Error for RetirementBusy {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RetirementPanic;
impl fmt::Display for RetirementPanic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("A retired payload destructor panicked")
    }
}
impl std::error::Error for RetirementPanic {}

/// Acknowledgement only: dropping this future does not cancel retirement.
pub(crate) struct Retirement {
    completion: Arc<Completion>,
}
#[derive(Default)]
struct Completion {
    state: Mutex<CompletionState>,
}
#[derive(Default)]
struct CompletionState {
    outcome: Option<Result<(), RetirementPanic>>,
    waker: Option<Waker>,
}

/// Async owners retain the slot even while their resource is not queued for cleanup.
/// Boxing at construction lets Drop transfer ownership without another allocation.
pub(crate) struct WorkerOwned<T: Send + 'static> {
    value: Option<Box<T>>,
    lease: Option<RetirementLease>,
}

impl RetirementDomain {
    pub(crate) fn try_start(capacity: usize, waiter_capacity: usize) -> io::Result<Self> {
        Self::start_with(capacity, waiter_capacity, |worker| {
            thread::Builder::new()
                .name("fm-retirement".into())
                .spawn(worker)
                .map(drop)
        })
    }

    fn start_with(
        capacity: usize,
        waiter_capacity: usize,
        spawn: impl FnOnce(Worker) -> io::Result<()>,
    ) -> io::Result<Self> {
        if capacity == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Retirement capacity must be positive",
            ));
        }
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                free: (0..capacity).rev().collect(),
                slots: (0..capacity).map(|_| None).collect(),
                pending: VecDeque::with_capacity(capacity),
                closing: false,
                free_waiters: (0..waiter_capacity).rev().collect(),
                waiters: (0..waiter_capacity).map(|_| None).collect(),
            }),
            ready: Condvar::new(),
            worker: OnceLock::new(),
        });
        let domain = Self {
            control: Arc::new(Control {
                shared: shared.clone(),
            }),
        };
        // Failure occurs before any lease/payload can escape to a caller.
        spawn(Box::new(move || run(shared)))?;
        Ok(domain)
    }

    pub(crate) fn new_waiter(&self) -> Result<RetirementWaiter, RetirementBusy> {
        let slot = self
            .control
            .shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .free_waiters
            .pop()
            .ok_or(RetirementBusy)?;
        Ok(RetirementWaiter {
            control: self.control.clone(),
            slot,
        })
    }

    pub(crate) fn try_reserve(&self) -> Result<RetirementLease, RetirementBusy> {
        let slot = self
            .control
            .shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .free
            .pop()
            .ok_or(RetirementBusy)?;
        Ok(RetirementLease {
            control: self.control.clone(),
            slot: Some(slot),
        })
    }
}

impl RetirementWaiter {
    /// Holds no payload; the caller must retain incoming ownership while pending.
    pub(crate) fn reserve(&mut self) -> impl Future<Output = RetirementLease> + '_ {
        Reservation {
            waiter: self,
            completed: false,
        }
    }
}
impl Future for Reservation<'_> {
    type Output = RetirementLease;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        assert!(!this.completed, "reservation polled after completion");
        let mut queue = this
            .waiter
            .control
            .shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(slot) = queue.free.pop() {
            let old = queue.waiters[this.waiter.slot].take();
            drop(queue);
            drop(old);
            this.completed = true;
            return Poll::Ready(RetirementLease {
                control: this.waiter.control.clone(),
                slot: Some(slot),
            });
        }
        let current = &mut queue.waiters[this.waiter.slot];
        if current
            .as_ref()
            .is_none_or(|waker| !waker.will_wake(context.waker()))
        {
            let old = current.replace(context.waker().clone());
            drop(queue);
            drop(old);
        }
        Poll::Pending
    }
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.completed {
            clear_waiter(&self.waiter.control.shared, self.waiter.slot);
        }
    }
}
impl Drop for RetirementWaiter {
    fn drop(&mut self) {
        let old = {
            let mut queue = self
                .control
                .shared
                .queue
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let old = queue.waiters[self.slot].take();
            queue.free_waiters.push(self.slot);
            old
        };
        drop(old);
    }
}
fn clear_waiter(shared: &Shared, slot: usize) {
    let old = shared
        .queue
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .waiters[slot]
        .take();
    drop(old);
}

impl Drop for Control {
    fn drop(&mut self) {
        self.shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closing = true;
        self.shared.ready.notify_one();
    }
}

impl RetirementLease {
    pub(crate) fn own<T: Send + 'static>(self, value: T) -> WorkerOwned<T> {
        WorkerOwned {
            value: Some(Box::new(value)),
            lease: Some(self),
        }
    }

    pub(crate) fn retire<T: Send + 'static>(self, value: T) -> Retirement {
        self.own(value).retire()
    }

    fn submit(mut self, payload: Payload, completion: Option<Arc<Completion>>) {
        let slot = self.slot.take().expect("lease is consumed once");
        let shared = &self.control.shared;
        let job = Job {
            payload,
            completion,
        };
        if shared.worker.get() == Some(&thread::current().id()) {
            // A destructor may contain another guarded owner in this domain.
            // Its reserved slot remains held throughout this nested destruction.
            destroy(shared, slot, job);
            return;
        }
        let mut queue = shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        debug_assert!(queue.slots[slot].is_none());
        queue.slots[slot] = Some(job);
        queue.pending.push_back(slot);
        drop(queue);
        shared.ready.notify_one();
    }
}
impl Drop for RetirementLease {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            release(&self.control.shared, slot);
        }
    }
}

impl<T: Send + 'static> WorkerOwned<T> {
    /// Worker-only handoff. Keep the lease until the raw value is destroyed, or
    /// immediately rewrap the transformed output; never release capacity early.
    pub(crate) fn into_parts(mut self) -> (T, RetirementLease) {
        let value = self.value.take().expect("owned value is consumed once");
        let lease = self.lease.take().expect("owned lease is consumed once");
        (*value, lease)
    }

    /// Enqueues immediately, before the returned acknowledgement is first polled.
    pub(crate) fn retire(mut self) -> Retirement {
        let completion = Arc::new(Completion::default());
        let value = self.value.take().expect("owned value is consumed once");
        let lease = self.lease.take().expect("owned lease is consumed once");
        lease.submit(value, Some(completion.clone()));
        Retirement { completion }
    }
}
impl<T: Send + 'static> Deref for WorkerOwned<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value.as_deref().expect("owned value is live")
    }
}
impl<T: Send + 'static> DerefMut for WorkerOwned<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value.as_deref_mut().expect("owned value is live")
    }
}
impl<T: Send + 'static> Drop for WorkerOwned<T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            let lease = self.lease.take().expect("live value retains its lease");
            lease.submit(value, None);
        }
    }
}

impl Future for Retirement {
    type Output = Result<(), RetirementPanic>;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self
            .completion
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(outcome) = state.outcome {
            return Poll::Ready(outcome);
        }
        if state
            .waker
            .as_ref()
            .is_none_or(|waker| !waker.will_wake(context.waker()))
        {
            let old = state.waker.replace(context.waker().clone());
            drop(state);
            drop(old);
        }
        Poll::Pending
    }
}
impl Completion {
    fn finish(&self, outcome: Result<(), RetirementPanic>) {
        let waker = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.outcome = Some(outcome);
            state.waker.take()
        };
        if let Some(waker) = waker {
            // An acknowledgement observer cannot terminate the cleanup worker.
            let _ = catch_unwind(AssertUnwindSafe(|| waker.wake()));
        }
    }
}

fn run(shared: Arc<Shared>) {
    let _ = shared.worker.set(thread::current().id());
    loop {
        let (slot, job) = {
            let mut queue = shared
                .queue
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            loop {
                if let Some(slot) = queue.pending.pop_front() {
                    break (
                        slot,
                        queue.slots[slot].take().expect("queued slot owns a job"),
                    );
                }
                if queue.closing {
                    return;
                }
                queue = shared
                    .ready
                    .wait(queue)
                    .unwrap_or_else(|error| error.into_inner());
            }
        };
        destroy(&shared, slot, job);
    }
}

fn destroy(shared: &Shared, slot: usize, job: Job) {
    let outcome = catch_unwind(AssertUnwindSafe(|| drop(job.payload))).map_err(|_| RetirementPanic);
    release(shared, slot);
    if let Some(completion) = job.completion {
        completion.finish(outcome);
    }
}

fn release(shared: &Shared, slot: usize) {
    shared
        .queue
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .free
        .push(slot);
    // Registration count is fixed at startup; never grow a list of async waiters.
    let count = shared
        .queue
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .waiters
        .len();
    for index in 0..count {
        let waker = shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .waiters[index]
            .take();
        if let Some(waker) = waker {
            let _ = catch_unwind(AssertUnwindSafe(|| waker.wake()));
        }
    }
}

#[cfg(test)]
#[path = "retirement_tests.rs"]
mod tests;
