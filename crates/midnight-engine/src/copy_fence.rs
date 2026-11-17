//! Bounded snapshots of outstanding copies; completed history is never retained globally.
use midnight_core::{Cancellation, CopyOperationState, CopyProgress, FsError};
use std::{
    io,
    path::Path,
    sync::{
        Arc, Mutex, MutexGuard, TryLockError, Weak,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

const PENDING: u8 = 0;
const SUCCESS: u8 = 1;
const FAILED: u8 = 2;

#[derive(Clone)]
pub(crate) struct CopyFence(Arc<Mutex<Vec<Weak<Completion>>>>);

impl Default for CopyFence {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Vec::with_capacity(
            crate::copy_operations::MAX_JOBS,
        ))))
    }
}

pub(crate) struct FenceRegistration<'a>(MutexGuard<'a, Vec<Weak<Completion>>>);

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
    #[cfg(test)]
    pub(crate) fn register(&self) -> Receipt {
        let receipt = Receipt::unpublished();
        let mut pending = self.0.lock().unwrap_or_else(|e| e.into_inner());
        pending.retain(|entry| {
            entry
                .upgrade()
                .is_some_and(|entry| entry.state.load(Ordering::Acquire) == PENDING)
        });
        pending.push(Arc::downgrade(&receipt.0));
        receipt
    }

    /// Refuse contention before either admission ledger publishes anything.
    pub(crate) fn try_registration(&self) -> Result<FenceRegistration<'_>, ()> {
        match self.0.try_lock() {
            Ok(pending) => Ok(FenceRegistration(pending)),
            Err(TryLockError::Poisoned(error)) => Ok(FenceRegistration(error.into_inner())),
            Err(TryLockError::WouldBlock) => Err(()),
        }
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

impl FenceRegistration<'_> {
    /// Caller owns admission capacity and holds the admission mutex. The receipt
    /// and bounded registry storage already exist; only metadata is published.
    pub(crate) fn publish(&mut self, receipt: &Receipt) {
        self.0.retain(|entry| {
            entry
                .upgrade()
                .is_some_and(|entry| entry.state.load(Ordering::Acquire) == PENDING)
        });
        assert!(self.0.len() < crate::copy_operations::MAX_JOBS);
        assert!(self.0.capacity() >= crate::copy_operations::MAX_JOBS);
        assert_eq!(receipt.0.state.load(Ordering::Acquire), PENDING);
        let weak = Arc::downgrade(&receipt.0);
        assert!(
            !self.0.iter().any(|entry| entry.ptr_eq(&weak)),
            "receipt was already published"
        );
        self.0.push(weak);
    }
}

impl Receipt {
    /// Allocate before attempting the admission and fence metadata locks.
    pub(crate) fn unpublished() -> Self {
        Self(Arc::new(Completion {
            state: AtomicU8::new(PENDING),
            ready: Notify::new(),
        }))
    }

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
    #[cfg(test)]
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

#[cfg(test)]
mod prepared_tests {
    use super::*;

    #[test]
    fn busy_registration_refuses_without_waiting_or_publishing() {
        let fence = CopyFence::default();
        let locked = fence.0.lock().unwrap();
        let other = fence.clone();
        let (sent, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let receipt = Receipt::unpublished();
            let refused = other.try_registration().is_err();
            sent.send(refused).unwrap();
            drop(receipt);
        });
        let refused_before_release = received.recv_timeout(Duration::from_secs(2));
        drop(locked);
        worker.join().unwrap();
        assert!(refused_before_release.unwrap());
        assert!(fence.snapshot().0.is_empty());
    }

    #[test]
    fn staged_publication_uses_preallocated_storage_and_prunes_completed_history() {
        let fence = CopyFence::default();
        let receipts: Vec<_> = (0..crate::copy_operations::MAX_JOBS)
            .map(|_| Receipt::unpublished())
            .collect();
        let storage = fence.0.lock().unwrap().as_ptr();
        assert!(fence.snapshot().0.is_empty());
        {
            let mut registration = fence.try_registration().unwrap();
            for receipt in &receipts {
                registration.publish(receipt);
            }
        }
        assert_eq!(fence.0.lock().unwrap().as_ptr(), storage);
        assert_eq!(fence.snapshot().0.len(), crate::copy_operations::MAX_JOBS);
        drop(receipts);
        let next = Receipt::unpublished();
        fence.try_registration().unwrap().publish(&next);
        assert_eq!(fence.0.lock().unwrap().as_ptr(), storage);
        assert_eq!(fence.snapshot().0.len(), 1);
        assert!(next.observer().is_pending());
    }

    #[test]
    fn poisoned_registration_lock_recovers_without_waiting() {
        let fence = CopyFence::default();
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _locked = fence.0.lock().unwrap();
            panic!("poison metadata registry");
        }));
        assert!(poisoned.is_err());
        let receipt = Receipt::unpublished();
        fence.try_registration().unwrap().publish(&receipt);
        assert_eq!(fence.snapshot().0.len(), 1);
    }

    #[tokio::test]
    async fn completed_receipt_fences_all_dirty_counter_and_error_combinations() {
        for (failed, skipped, has_error, allows_mutation) in [
            (0, 0, false, true),
            (0, 0, true, false),
            (0, 1, false, false),
            (0, 1, true, false),
            (1, 0, false, false),
            (1, 0, true, false),
            (1, 1, false, false),
            (1, 1, true, false),
        ] {
            let fence = CopyFence::default();
            let receipt = Receipt::unpublished();
            fence.try_registration().unwrap().publish(&receipt);
            let before = fence.snapshot();
            assert_eq!(before.0.len(), 1);
            let mut progress = CopyProgress::queued(
                midnight_core::OperationId(1),
                Default::default(),
                1,
                midnight_core::TransferIntent::Copy,
            );
            progress.state = CopyOperationState::Completed;
            progress.failed = failed;
            progress.skipped = skipped;
            if has_error {
                progress.errors.push(FsError::from_io(
                    "controlled copy fence failure",
                    Path::new(""),
                    io::Error::other("controlled test error"),
                ));
            }
            receipt.finish(&progress);
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                before.wait(&Cancellation::default(), Path::new("")),
            )
            .await
            .expect("a finished receipt must resolve its captured predecessors");
            assert_eq!(
                result.is_ok(),
                allows_mutation,
                "failed={failed}, skipped={skipped}, has_error={has_error}: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn staged_receipt_preserves_success_and_failure_fence_results() {
        let fence = CopyFence::default();
        let receipt = Receipt::unpublished();
        fence.try_registration().unwrap().publish(&receipt);
        let before = fence.snapshot();
        let mut progress = CopyProgress::queued(
            midnight_core::OperationId(1),
            Default::default(),
            1,
            midnight_core::TransferIntent::Copy,
        );
        progress.state = CopyOperationState::Completed;
        receipt.finish(&progress);
        assert!(
            before
                .wait(&Cancellation::default(), Path::new(""))
                .await
                .is_ok()
        );
        let abandoned = Receipt::unpublished();
        fence.try_registration().unwrap().publish(&abandoned);
        let failure = fence.snapshot();
        drop(abandoned);
        assert!(
            failure
                .wait(&Cancellation::default(), Path::new(""))
                .await
                .is_err()
        );
    }
}
