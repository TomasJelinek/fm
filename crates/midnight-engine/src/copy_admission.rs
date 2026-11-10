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
        // Close sends synchronously; defer only payload destruction.
        // One bounded shutdown batch, including oneshot payloads that arrived
        // after cancellation. Their receiver must also die on a blocking worker.
        tokio::task::spawn_blocking(move || {
            for mut slot in retired {
                if let Ok(request) = slot.payload.try_recv() {
                    drop(request.input.into_inner());
                }
            }
        });
    }
}

pub(crate) struct Reservation {
    receipt: Receipt,
    payload: oneshot::Sender<Request>,
    // Resolve abandonment and close the slot before another reservation can enter.
    capacity: OwnedSemaphorePermit,
}

impl Default for Admission {
    fn default() -> Self {
        Self(Arc::new(Shared {
            slots: Mutex::default(),
            ready: Notify::new(),
            capacity: Arc::new(Semaphore::new(MAX_JOBS)),
            fence: CopyFence::default(),
        }))
    }
}

impl Admission {
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
        Ok(Reservation {
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
    pub(super) fn send(self, input: Input) -> Result<(), Box<Request>> {
        let Self {
            capacity,
            receipt,
            payload,
        } = self;
        payload
            .send(Request {
                _capacity: capacity,
                receipt,
                input: WorkerOwned::new(input),
            })
            .map_err(Box::new)
    }
}
