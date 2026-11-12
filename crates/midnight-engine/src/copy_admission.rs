//! FIFO slots are allocated at native/backend reservation, before any UI handoff.
use super::*;
use crate::copy_fence::{CopyFence, PendingReceipt, Receipt};
use std::collections::VecDeque;
use tokio::sync::{Notify, oneshot};

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
    slots: Mutex<VecDeque<Slot>>,
    ready: Notify,
    capacity: Arc<Semaphore>,
    fence: CopyFence,
    retirement: crate::retirement::RetirementDomain,
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
        let mut retired =
            std::mem::take(&mut *self.0.0.slots.lock().unwrap_or_else(|e| e.into_inner()));
        for slot in &mut retired {
            slot.payload.close();
        }
        // Every sent payload already carries its own reserved cleanup slot and
        // capacity. Closing/dropping receivers only drops those guard envelopes.
        drop(retired);
    }
}

pub(crate) struct Reservation {
    slot: SlotReservation,
    lease: crate::retirement::RetirementLease,
}

pub(super) struct SlotReservation {
    receipt: Receipt,
    payload: oneshot::Sender<Request>,
    capacity: OwnedSemaphorePermit,
}

#[cfg(test)]
impl Default for Admission {
    fn default() -> Self {
        Self::new(crate::retirement::RetirementDomain::try_start(MAX_JOBS, 0).unwrap())
    }
}

impl Admission {
    pub(super) fn new(retirement: crate::retirement::RetirementDomain) -> Self {
        Self(Arc::new(Shared {
            slots: Mutex::default(),
            ready: Notify::new(),
            capacity: Arc::new(Semaphore::new(MAX_JOBS)),
            fence: CopyFence::default(),
            retirement,
        }))
    }
    pub(crate) fn capacity(&self) -> Arc<Semaphore> {
        self.0.capacity.clone()
    }
    pub(crate) fn fence(&self) -> CopyFence {
        self.0.fence.clone()
    }
    pub(super) fn consumer(&self) -> Consumer {
        Consumer(self.clone())
    }

    pub(crate) fn reserve(&self) -> Result<Reservation, FsError> {
        let slot = self.reserve_slot()?;
        let lease = self.0.retirement.try_reserve().map_err(|_| {
            error(
                Path::new(""),
                io::ErrorKind::WouldBlock,
                "Copy retirement capacity is occupied.",
            )
        })?;
        Ok(Reservation { slot, lease })
    }

    pub(super) fn reserve_slot(&self) -> Result<SlotReservation, FsError> {
        let mut slots = self.0.slots.lock().unwrap_or_else(|e| e.into_inner());
        let capacity = self.0.capacity.clone().try_acquire_owned().map_err(|_| {
            error(
                Path::new(""),
                io::ErrorKind::WouldBlock,
                "The copy queue is full or closing (32 jobs).",
            )
        })?;
        // Repeated rejected native drops cannot accumulate abandoned placeholders.
        slots.retain(|slot| {
            slot.pending
                .as_ref()
                .is_some_and(PendingReceipt::is_pending)
        });
        let receipt = self.0.fence.register();
        let (payload, receiver) = oneshot::channel();
        slots.push_back(Slot {
            payload: receiver,
            pending: Some(receipt.observer()),
        });
        self.0.ready.notify_one();
        Ok(SlotReservation {
            capacity,
            receipt,
            payload,
        })
    }

    pub(super) async fn next(&self, cancel: &Cancellation) -> Option<Request> {
        loop {
            let notified = self.0.ready.notified();
            let slot = self
                .0
                .slots
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop_front();
            if let Some(mut slot) = slot {
                loop {
                    match slot.payload.try_recv() {
                        Ok(request) => {
                            slot.pending.take();
                            return Some(request);
                        }
                        Err(oneshot::error::TryRecvError::Closed) => break,
                        Err(oneshot::error::TryRecvError::Empty) => {}
                    }
                    if cancel.is_cancelled() {
                        break;
                    }
                    tokio::select! {
                        payload = &mut slot.payload => {
                            if let Ok(request) = payload { slot.pending.take(); return Some(request); }
                            break;
                        },
                        _ = tokio::time::sleep(Duration::from_millis(25)) => {},
                    }
                }
                continue;
            }
            if cancel.is_cancelled() {
                return None;
            }
            tokio::select! { _ = notified => {}, _ = tokio::time::sleep(Duration::from_millis(25)) => {} }
        }
    }
}

impl Reservation {
    pub(super) fn attach<T: Send + 'static>(
        self,
        input: T,
    ) -> (Owned<T>, oneshot::Sender<Request>) {
        self.slot.attach(self.lease.own(input))
    }
}
impl SlotReservation {
    pub(super) fn attach<T: Send + 'static>(
        self,
        input: WorkerOwned<T>,
    ) -> (Owned<T>, oneshot::Sender<Request>) {
        let Self {
            capacity,
            receipt,
            payload,
        } = self;
        let (input, lease) = input.into_parts();
        let owned = lease.own(Scope {
            payload: input,
            resources: ownership::Resources {
                receipt,
                mutation: None,
                _capacity: capacity,
            },
        });
        (owned, payload)
    }
}
