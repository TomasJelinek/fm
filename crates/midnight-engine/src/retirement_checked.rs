//! Checked ownership retains its original indexed slot until an explicit receipt.
use super::super::copy_nested_retirement::Context as ReleaseContext;
use super::*;
use midnight_core::FsError;

pub(crate) enum CheckedRelease<R> {
    Confirmed(R),
    Unconfirmed(FsError),
}
pub(crate) enum CheckedFinish<T, A, R> {
    Confirmed {
        payload: T,
        accounting: A,
        result: R,
    },
    Unconfirmed(FsError),
}

/// Qualification promises that Confirmed leaves only provider-free fields.
/// Unknown implementations cannot opt in by merely returning from Drop.
pub(crate) trait CheckedPayload: Send + 'static {
    fn retirement_is_checked(&self) -> bool {
        false
    }
    fn discard_on_worker(&mut self) -> CheckedRelease<()>;
}

pub(crate) struct CheckedWorkerOwned<T: CheckedPayload, A: Send + 'static> {
    frame: Option<Box<Frame<T, A>>>,
}
struct Frame<T: CheckedPayload, A: Send + 'static> {
    payload: Option<T>,
    accounting: Option<A>,
    lease: Option<RetirementLease>,
    context: ReleaseContext,
    completion: Arc<CheckedCompletion>,
}

/// Already queued; observing or dropping this future has no ownership effect.
pub(crate) struct CheckedRetirement {
    completion: Arc<CheckedCompletion>,
}
#[derive(Default)]
pub(super) struct CheckedCompletion {
    state: Mutex<CheckedCompletionState>,
}
#[derive(Default)]
struct CheckedCompletionState {
    outcome: Option<Result<(), FsError>>,
    waker: Option<Waker>,
}

pub(super) trait CheckedJob: Send {
    fn discard(&mut self) -> Result<(), FsError>;
    fn completion(&self) -> Arc<CheckedCompletion>;
    fn release_fields(&mut self);
}

impl RetirementLease {
    /// All frame and acknowledgement allocation happens before capture begins.
    pub(crate) fn own_checked<T: CheckedPayload, A: Send + 'static>(
        self,
        payload: T,
        accounting: A,
    ) -> CheckedWorkerOwned<T, A> {
        CheckedWorkerOwned {
            frame: Some(Box::new(Frame {
                payload: Some(payload),
                accounting: Some(accounting),
                lease: Some(self),
                context: ReleaseContext::default(),
                completion: Arc::new(CheckedCompletion::default()),
            })),
        }
    }
}

impl<T: CheckedPayload, A: Send + 'static> CheckedWorkerOwned<T, A> {
    /// Backend-only access; owned fields and actual reservation stay outside unwind.
    pub(crate) fn with_worker(
        &mut self,
        operation: impl FnOnce(&mut T) -> Result<(), FsError>,
    ) -> Result<Result<(), FsError>, RetirementPanic> {
        let frame = self.frame.as_mut().expect("checked frame remains armed");
        let context = frame.context.clone();
        match catch_unwind(AssertUnwindSafe(|| {
            context.with(|| {
                operation(
                    frame
                        .payload
                        .as_mut()
                        .expect("checked payload remains armed"),
                )
            })
        })) {
            Ok(result) => Ok(result),
            Err(panic) => {
                std::mem::forget(panic);
                frame.context.quarantine();
                Err(RetirementPanic)
            }
        }
    }

    pub(crate) fn finish_on_worker<R>(
        mut self,
        operation: impl FnOnce(&mut T) -> CheckedRelease<R>,
    ) -> CheckedFinish<T, A, R> {
        let mut frame = self.frame.take().expect("checked frame consumed once");
        match frame.finish(operation) {
            Ok(result) => {
                let payload = frame.payload.take().expect("confirmed payload exists");
                let accounting = frame
                    .accounting
                    .take()
                    .expect("confirmed accounting exists");
                // No capture/native owner remains in either returned value.
                drop(frame.lease.take());
                CheckedFinish::Confirmed {
                    payload,
                    accounting,
                    result,
                }
            }
            Err(error) => {
                quarantine(frame);
                CheckedFinish::Unconfirmed(error)
            }
        }
    }

    pub(crate) fn retire(mut self) -> CheckedRetirement {
        let frame = self.frame.take().expect("checked frame consumed once");
        let completion = frame.completion.clone();
        enqueue(frame);
        CheckedRetirement { completion }
    }
}
impl<T: CheckedPayload, A: Send + 'static> Drop for CheckedWorkerOwned<T, A> {
    fn drop(&mut self) {
        if let Some(frame) = self.frame.take() {
            enqueue(frame);
        }
    }
}

impl<T: CheckedPayload, A: Send + 'static> Frame<T, A> {
    fn finish<R>(
        &mut self,
        operation: impl FnOnce(&mut T) -> CheckedRelease<R>,
    ) -> Result<R, FsError> {
        if self.context.is_quarantined() {
            return Err(unconfirmed());
        }
        let context = self.context.clone();
        let result = catch_unwind(AssertUnwindSafe(|| {
            context.with(|| {
                let payload = self
                    .payload
                    .as_mut()
                    .expect("checked payload remains armed");
                if !payload.retirement_is_checked() {
                    return CheckedRelease::Unconfirmed(unconfirmed());
                }
                operation(payload)
            })
        }));
        match result {
            Ok(CheckedRelease::Confirmed(result)) if !self.context.is_quarantined() => Ok(result),
            Ok(CheckedRelease::Unconfirmed(error)) => {
                self.context.quarantine();
                Err(error)
            }
            Ok(CheckedRelease::Confirmed(_)) => Err(unconfirmed()),
            Err(panic) => {
                std::mem::forget(panic);
                self.context.quarantine();
                Err(unconfirmed())
            }
        }
    }
}
impl<T: CheckedPayload, A: Send + 'static> CheckedJob for Frame<T, A> {
    fn discard(&mut self) -> Result<(), FsError> {
        self.finish(T::discard_on_worker)
    }
    fn completion(&self) -> Arc<CheckedCompletion> {
        self.completion.clone()
    }
    fn release_fields(&mut self) {
        drop(self.payload.take());
        drop(self.accounting.take());
        drop(self.lease.take());
    }
}

// Defensive fallback: an unforeseen owner destruction never releases an armed
// native field or advertises its slot free. Normal paths empty confirmed fields
// or retain the complete frame in the original slot below.
impl<T: CheckedPayload, A: Send + 'static> Drop for Frame<T, A> {
    fn drop(&mut self) {
        if let Some(payload) = self.payload.take() {
            std::mem::forget(payload);
        }
        if let Some(accounting) = self.accounting.take() {
            std::mem::forget(accounting);
        }
        if let Some(lease) = self.lease.take() {
            std::mem::forget(lease);
        }
    }
}

fn enqueue<T: CheckedPayload, A: Send + 'static>(frame: Box<Frame<T, A>>) {
    let lease = frame
        .lease
        .as_ref()
        .expect("armed frame owns its original lease");
    let shared = lease.control.shared.clone();
    let slot = lease.slot.expect("armed lease owns a slot");
    let job: Box<dyn CheckedJob> = frame;
    if shared.worker.get() == Some(&thread::current().id()) {
        destroy(&shared, slot, job);
        return;
    }
    let mut queue = shared
        .queue
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    debug_assert!(queue.slots[slot].is_none());
    queue.slots[slot] = Some(Job::Checked(job));
    queue.pending.push_back(slot);
    drop(queue);
    shared.ready.notify_one();
}

fn quarantine<T: CheckedPayload, A: Send + 'static>(frame: Box<Frame<T, A>>) {
    let lease = frame
        .lease
        .as_ref()
        .expect("quarantined frame owns its lease");
    let shared = lease.control.shared.clone();
    let slot = lease.slot.expect("quarantined lease owns a slot");
    retain(&shared, slot, frame);
}
fn retain(shared: &Shared, slot: usize, job: Box<dyn CheckedJob>) {
    let mut queue = shared
        .queue
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    debug_assert!(queue.slots[slot].is_none());
    // The frame retains Control -> Shared -> this actual slot. Closing cannot
    // destroy it, and no pending entry retries uncertain native teardown.
    queue.slots[slot] = Some(Job::Checked(job));
}

pub(super) fn destroy(shared: &Shared, slot: usize, mut job: Box<dyn CheckedJob>) {
    let completion = job.completion();
    match job.discard() {
        Ok(()) => match catch_unwind(AssertUnwindSafe(|| job.release_fields())) {
            Ok(()) => {
                drop(job);
                completion.finish(Ok(()));
            }
            Err(panic) => {
                std::mem::forget(panic);
                retain(shared, slot, job);
                completion.finish(Err(unconfirmed()));
            }
        },
        Err(error) => {
            retain(shared, slot, job);
            completion.finish(Err(error));
        }
    }
}

impl Future for CheckedRetirement {
    type Output = Result<(), FsError>;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self
            .completion
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(result) = &state.outcome {
            return Poll::Ready(result.clone());
        }
        if state
            .waker
            .as_ref()
            .is_none_or(|old| !old.will_wake(context.waker()))
        {
            let old = state.waker.replace(context.waker().clone());
            drop(state);
            drop(old);
        }
        Poll::Pending
    }
}
impl CheckedCompletion {
    fn finish(&self, outcome: Result<(), FsError>) {
        let waker = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.outcome = Some(outcome);
            state.waker.take()
        };
        if let Some(waker) = waker
            && let Err(panic) = catch_unwind(AssertUnwindSafe(|| waker.wake()))
        {
            std::mem::forget(panic);
        }
    }
}
fn unconfirmed() -> FsError {
    FsError::from_io(
        "checked retirement",
        std::path::Path::new(""),
        io::Error::other("Native cleanup is unconfirmed; its admission remains retained."),
    )
}
