//! Bounded snapshots of outstanding copies; completed history is never retained globally.
use midnight_core::{Cancellation, CopyOperationState, CopyProgress, FsError};
use std::{
    io,
    path::Path,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

const PENDING: u8 = 0;
const SUCCESS: u8 = 1;
const FAILED: u8 = 2;

#[derive(Clone, Default)]
pub(crate) struct CopyFence(Arc<Mutex<Vec<Weak<Completion>>>>);

struct Completion {
    state: AtomicU8,
    ready: Notify,
}

pub(crate) struct Receipt(Arc<Completion>);
pub(crate) struct PendingReceipt(Weak<Completion>);

#[derive(Default)]
pub(crate) struct Predecessors(Vec<Arc<Completion>>);

impl CopyFence {
    /// The caller must already own one of the shared 32 copy permits.
    pub(crate) fn register(&self) -> Receipt {
        let mut pending = self.0.lock().unwrap_or_else(|e| e.into_inner());
        pending.retain(|entry| {
            entry
                .upgrade()
                .is_some_and(|entry| entry.state.load(Ordering::Acquire) == PENDING)
        });
        let completion = Arc::new(Completion {
            state: AtomicU8::new(PENDING),
            ready: Notify::new(),
        });
        pending.push(Arc::downgrade(&completion));
        Receipt(completion)
    }

    pub(crate) fn snapshot(&self) -> Predecessors {
        Predecessors(
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter_map(Weak::upgrade)
                .filter(|entry| entry.state.load(Ordering::Acquire) == PENDING)
                .collect(),
        )
    }
}

impl Receipt {
    pub(crate) fn finish(&self, progress: &CopyProgress) {
        let success = progress.state == CopyOperationState::Completed
            && progress.failed == 0
            && progress.skipped == 0
            && progress.errors.is_empty();
        self.resolve(if success { SUCCESS } else { FAILED });
    }

    fn resolve(&self, outcome: u8) {
        if self
            .0
            .state
            .compare_exchange(PENDING, outcome, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.0.ready.notify_waiters();
        }
    }

    pub(crate) fn observer(&self) -> PendingReceipt {
        PendingReceipt(Arc::downgrade(&self.0))
    }
}

impl PendingReceipt {
    pub(crate) fn is_pending(&self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|entry| entry.state.load(Ordering::Acquire) == PENDING)
    }

    pub(crate) fn fail(&self) {
        if let Some(completion) = self.0.upgrade() {
            Receipt(completion).resolve(FAILED);
        }
    }
}

impl Drop for Receipt {
    fn drop(&mut self) {
        self.resolve(FAILED);
    }
}

impl Predecessors {
    pub(crate) async fn wait(&self, cancel: &Cancellation, path: &Path) -> Result<(), FsError> {
        for completion in &self.0 {
            loop {
                let notified = completion.ready.notified();
                if cancel.is_cancelled() {
                    return Err(FsError::from_io(
                        "wait for earlier copies",
                        path,
                        io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Operation cancelled while waiting for earlier copies; no mutation was started.",
                        ),
                    ));
                }
                match completion.state.load(Ordering::Acquire) {
                    SUCCESS => break,
                    FAILED => {
                        return Err(FsError::from_io(
                            "wait for earlier copies",
                            path,
                            io::Error::other(
                                "An earlier copy did not complete fully verified publication. No mutation was started. Inspect the copy results and confirm the operation again.",
                            ),
                        ));
                    }
                    _ => {
                        tokio::select! { _ = notified => {}, _ = tokio::time::sleep(Duration::from_millis(25)) => {} }
                    }
                }
            }
        }
        Ok(())
    }
}
