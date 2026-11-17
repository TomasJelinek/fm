//! Ordered reservations with bounded ready scanning before any provider preparation.
use super::*;
#[cfg(test)]
use crate::copy_fence::Receipt;
use crate::copy_fence::{CopyFence, PendingReceipt};
use std::collections::VecDeque;
use tokio::sync::{Notify, oneshot};
#[path = "copy_drop_admission.rs"]
mod drop_admission;

struct Slot {
    payload: oneshot::Receiver<Request>,
    pending: Option<PendingReceipt>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            pending.fail();
        }
    }
}

struct Shared {
    state: Mutex<State>,
    ready: Notify,
    capacity: Arc<Semaphore>,
    fence: CopyFence,
    retirement: crate::retirement::RetirementDomain,
    cancel: Cancellation,
}

struct State {
    slots: VecDeque<Slot>,
    finished: crate::copy_finished::Registry,
    controls: Vec<PendingControl>,
}
struct PendingControl {
    id: OperationId,
    cancel: Cancellation,
    finished: crate::copy_finished::FinishedObserver,
}
impl Default for State {
    fn default() -> Self {
        Self {
            slots: VecDeque::with_capacity(MAX_JOBS),
            finished: crate::copy_finished::Registry::default(),
            controls: Vec::with_capacity(MAX_JOBS),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Admission(Arc<Shared>);

pub(super) struct Consumer(Admission);

impl Consumer {
    pub(super) fn admission(&self) -> &Admission {
        &self.0
    }
}

impl Drop for Consumer {
    fn drop(&mut self) {
        self.0.0.capacity.close();
        let mut retired = std::mem::take(
            &mut self
                .0
                .0
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .slots,
        );
        for slot in &mut retired {
            slot.payload.close();
        }
        // Every sent payload already carries its own reserved cleanup slot and
        // capacity. Closing/dropping receivers only drops those guard envelopes.
        drop(retired);
    }
}

pub(crate) struct Reservation {
    owned: Owned<()>,
    payload: oneshot::Sender<Request>,
    completions: Option<Arc<HashMap<std::path::PathBuf, TreeCopyOccurrence>>>,
}

#[cfg(test)]
pub(super) struct SlotReservation {
    receipt: Receipt,
    payload: oneshot::Sender<Request>,
    capacity: OwnedSemaphorePermit,
    finished: crate::copy_finished::Owner,
    predecessors: crate::copy_finished::Predecessors,
}

#[cfg(test)]
impl Default for Admission {
    fn default() -> Self {
        Self::new(crate::retirement::RetirementDomain::try_start(MAX_JOBS, 0).unwrap())
    }
}

impl Admission {
    pub(crate) fn new(retirement: crate::retirement::RetirementDomain) -> Self {
        Self(Arc::new(Shared {
            state: Mutex::default(),
            ready: Notify::new(),
            capacity: Arc::new(Semaphore::new(MAX_JOBS)),
            fence: CopyFence::default(),
            retirement,
            cancel: Cancellation::default(),
        }))
    }
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> Arc<Semaphore> {
        self.0.capacity.clone()
    }
    pub(crate) fn fence(&self) -> CopyFence {
        self.0.fence.clone()
    }
    pub(crate) fn cancellation(&self) -> Cancellation {
        self.0.cancel.clone()
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(crate) fn cancel_pending(&self, id: OperationId) {
        let state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for control in &state.controls {
            if control.id == id {
                control.cancel.cancel();
            }
        }
    }
    pub(crate) fn shutdown(&self) {
        let _state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.0.cancel.cancel();
        self.0.capacity.close();
    }
    pub(super) fn consumer(&self) -> Consumer {
        Consumer(self.clone())
    }

    #[cfg(test)]
    pub(crate) fn reserve(&self) -> Result<Reservation, FsError> {
        self.stage_drop()?.publish()
    }

    #[cfg(test)]
    pub(super) fn reserve_slot(&self) -> Result<SlotReservation, FsError> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        self.reserve_slot_locked(&mut state)
    }

    #[cfg(test)]
    fn reserve_slot_locked(&self, state: &mut State) -> Result<SlotReservation, FsError> {
        let capacity = self.0.capacity.clone().try_acquire_owned().map_err(|_| {
            error(
                Path::new(""),
                io::ErrorKind::WouldBlock,
                "The copy queue is full or closing (32 jobs).",
            )
        })?;
        // Closed holes are removed by the consumer outside this mutex. They must
        // not permit the reservation queue itself to exceed the admission bound.
        if state.slots.len() >= MAX_JOBS {
            return Err(error(
                Path::new(""),
                io::ErrorKind::WouldBlock,
                "The copy reservation queue is full.",
            ));
        }
        let (finished, predecessors) = state
            .finished
            .register()
            .map_err(|message| error(Path::new(""), io::ErrorKind::WouldBlock, message))?;
        let receipt = self.0.fence.register();
        let (payload, receiver) = oneshot::channel();
        state.slots.push_back(Slot {
            payload: receiver,
            pending: Some(receipt.observer()),
        });
        self.0.ready.notify_one();
        Ok(SlotReservation {
            capacity,
            receipt,
            payload,
            finished,
            predecessors,
        })
    }

    /// Publish the complete guarded command and its admission as one ordered act.
    /// The channel permit is already reserved. No foreign accessor runs here.
    #[expect(
        clippy::result_large_err,
        reason = "Keep the original refused command inline without an additional box allocation."
    )]
    pub(crate) fn send_reserved(
        &self,
        command: Command,
        permit: mpsc::OwnedPermit<crate::engine_commands::Message>,
    ) -> Result<(), mpsc::error::TrySendError<Command>> {
        use mpsc::error::TrySendError;
        let refused = |command| {
            if self.0.capacity.is_closed() {
                TrySendError::Closed(command)
            } else {
                TrySendError::Full(command)
            }
        };
        let lease = match self.0.retirement.try_reserve_nowait() {
            Ok(lease) => lease,
            Err(_) => return Err(refused(command)),
        };
        let capacity = match self.0.capacity.clone().try_acquire_owned() {
            Ok(capacity) => capacity,
            Err(_) => return Err(refused(command)),
        };
        let cancel = self.0.cancel.pausable_child();
        let resources = ownership::Resources::unpublished(capacity, cancel.clone());
        let mut staged = lease.own(Scope::new(submission::Submission::new(command), resources));
        let (payload, receiver) = oneshot::channel();
        let slot = Slot {
            payload: receiver,
            pending: Some(staged.resources_mut().receipt_observer()),
        };
        #[cfg(test)]
        faults::at(faults::Point::AfterArmBeforeLock);
        // The staged guard was declared BEFORE either mutex guard. An unwind
        // unlocks first, then retires the original full command on its worker.
        let mut state = match self.0.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(refused(ownership::rollback_submission(staged)));
            }
        };
        if self.0.capacity.is_closed() || state.slots.len() >= MAX_JOBS {
            drop(state);
            return Err(refused(ownership::rollback_submission(staged)));
        }
        let sequence = match state.finished.prepare_sequence() {
            Ok(sequence) => sequence,
            Err(_) => {
                drop(state);
                return Err(refused(ownership::rollback_submission(staged)));
            }
        };
        let mut fence = match self.0.fence.try_registration() {
            Ok(fence) => fence,
            Err(()) => {
                drop(state);
                return Err(refused(ownership::rollback_submission(staged)));
            }
        };
        state
            .controls
            .retain(|control| !control.finished.is_finished());
        staged
            .resources_mut()
            .publish_admission(sequence, &mut state.finished, &mut fence);
        let finished = staged.resources_mut().finished_observer();
        state.controls.push(PendingControl {
            id: staged.id,
            cancel,
            finished,
        });
        state.slots.push_back(slot);
        #[cfg(test)]
        faults::at(faults::Point::AfterRegisterBeforePublish);
        permit.send(crate::engine_commands::Message::GuardedCopy(Admitted {
            request: staged,
            sender: payload,
        }));
        // A later predecessor snapshot cannot see this latch until its original
        // payload is armed AND its infallible FIFO handoff has completed.
        drop(fence);
        drop(state);
        self.0.ready.notify_one();
        Ok(())
    }

    pub(super) async fn next(&self, cancel: &Cancellation) -> Option<Request> {
        loop {
            if cancel.is_cancelled() {
                return None;
            }
            let notified = self.0.ready.notified();
            let mut retired = Vec::new();
            let mut ready = None;
            {
                let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
                let mut index = 0;
                while index < state.slots.len() {
                    match state.slots[index].payload.try_recv() {
                        Ok(request) => {
                            let mut slot = state.slots.remove(index).expect("scanned slot exists");
                            slot.pending.take();
                            retired.push(slot);
                            ready = Some(request);
                            break;
                        }
                        Err(oneshot::error::TryRecvError::Closed) => {
                            retired.push(state.slots.remove(index).expect("scanned slot exists"));
                        }
                        Err(oneshot::error::TryRecvError::Empty) => index += 1,
                    }
                }
            }
            // Receiver and old SUCCESS-fence observer destruction is outside the
            // admission mutex. Neither observer can resolve the FINISHED latch.
            drop(retired);
            if let Some(request) = ready {
                return Some(request);
            }
            tokio::select! { _ = notified => {}, _ = tokio::time::sleep(Duration::from_millis(25)) => {} }
        }
    }
}

impl Reservation {
    pub(super) fn attach_input(
        self,
        input: CopyInput,
    ) -> (Owned<CopyInput>, oneshot::Sender<Request>) {
        let Self {
            owned,
            payload,
            completions: _,
        } = self;
        (ownership::attach_reserved_input(owned, input), payload)
    }
    pub(crate) fn completions(
        &self,
    ) -> Option<Arc<HashMap<std::path::PathBuf, TreeCopyOccurrence>>> {
        self.completions.clone()
    }
    #[cfg(test)]
    pub(super) fn attach<T: crate::copy_nested_retirement::CheckedRetirement + 'static>(
        self,
        input: T,
    ) -> Result<(Owned<T>, oneshot::Sender<Request>), ownership::Quarantined> {
        let Self {
            owned,
            payload,
            completions: _,
        } = self;
        ownership::map(owned, |_| input).map(|owned| (owned, payload))
    }
}
#[cfg(test)]
impl SlotReservation {
    pub(super) fn attach<T: crate::copy_nested_retirement::CheckedRetirement + 'static>(
        self,
        input: WorkerOwned<T>,
    ) -> (Owned<T>, oneshot::Sender<Request>) {
        let Self {
            capacity,
            receipt,
            payload,
            finished,
            predecessors,
        } = self;
        let (input, lease) = input.into_parts();
        let resources = ownership::Resources::admission(receipt, finished, capacity, predecessors);
        let owned = lease.own(Scope::new(input, resources));
        (owned, payload)
    }
}

#[cfg(test)]
#[path = "copy_admission_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "copy_submission_faults.rs"]
mod faults;
#[cfg(test)]
#[path = "copy_admission_transport_tests.rs"]
mod transport_tests;
