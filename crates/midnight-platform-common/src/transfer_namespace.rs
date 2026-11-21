//! Synchronous native namespace transitions, separate from data I/O and private writes.
use midnight_core::{Cancellation, FsError, TransferNamespacePin};
use std::{cell::RefCell, sync::Arc};

pub trait TransferNamespaceChanges: Send + Sync {
    fn begin(&self, cancel: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError>;
}

pub trait TransferNamespaceChange: Send {
    /// Reserve a prospective private name before native creation/capture. False means
    /// choose a fresh nonce; never wait for a peer while retaining the mutation turn.
    /// The host retains a pin it accepts at least until this transaction finishes, so an
    /// adapter may identify it by address within the transaction.
    fn reserve_private_child(
        &mut self,
        pin: Option<Arc<dyn TransferNamespacePin>>,
        cancel: &Cancellation,
    ) -> Result<bool, FsError>;
    /// Refresh owned transitions on every ordinary Result, including failure/cancellation.
    /// The host latches refresh errors as Opaque plus a retained diagnostic before Err;
    /// an already committed native outcome must keep its separate operation result.
    /// Uncommitted Drop means native state is unconfirmed and must quarantine the claim.
    fn finish(self: Box<Self>) -> Result<(), FsError>;
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<dyn TransferNamespaceChanges>>> = const { RefCell::new(None) };
}

struct Restore(Option<Arc<dyn TransferNamespaceChanges>>);
impl Drop for Restore {
    fn drop(&mut self) {
        let retired = CURRENT.with(|current| current.replace(self.0.take()));
        drop(retired);
    }
}

/// Install only for one synchronous backend call. Never retain this context across an await.
pub fn with_transfer_namespace_changes<R>(
    changes: Arc<dyn TransferNamespaceChanges>,
    operation: impl FnOnce() -> R,
) -> R {
    let previous = CURRENT.with(|current| current.replace(Some(changes)));
    let _restore = Restore(previous);
    operation()
}

/// None means the legacy caller must already hold its ordinary exclusive authority.
/// Absence of a Copy context grants no additional native access or fallback.
pub fn begin_transfer_namespace_change(
    cancel: &Cancellation,
) -> Result<Option<Box<dyn TransferNamespaceChange>>, FsError> {
    let changes = current_transfer_namespace_changes();
    changes.map(|changes| changes.begin(cancel)).transpose()
}

/// Clone the installed adapter without borrowing TLS across provider code. Native adapters
/// may use its weak identity to share mutable witnesses within one admitted job. Pins must
/// never retain a strong reference back to this adapter.
pub fn current_transfer_namespace_changes() -> Option<Arc<dyn TransferNamespaceChanges>> {
    CURRENT.with(|current| current.borrow().clone())
}

thread_local! {
    static REFRESH_BATCH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
static REFRESH_BATCHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One batch of namespace pin refreshes on this thread, such as a host refreshing every
/// pin that a change touched, or an adapter reserving the pins of one announcement. A
/// batch contains no namespace change, so an adapter may check an ancestry that several
/// of its pins share once for the whole batch instead of once per pin. The batch ends
/// when this drops, on the thread that began it; a nested batch is a separate batch and
/// must end first (keep each guard in a scope, as `let _batch = ...begin();`).
pub struct TransferNamespaceRefreshBatch {
    previous: u64,
    _thread: std::marker::PhantomData<*const ()>,
}

impl TransferNamespaceRefreshBatch {
    pub fn begin() -> Self {
        let batch = REFRESH_BATCHES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        Self {
            previous: REFRESH_BATCH.with(|current| current.replace(batch)),
            _thread: std::marker::PhantomData,
        }
    }
}

impl Drop for TransferNamespaceRefreshBatch {
    fn drop(&mut self) {
        REFRESH_BATCH.with(|current| current.set(self.previous));
    }
}

/// The number of the refresh batch running on this thread, unique to that batch (see
/// `TransferNamespaceRefreshBatch`).
pub fn current_transfer_namespace_refresh_batch() -> Option<u64> {
    REFRESH_BATCH.with(|current| Some(current.get()).filter(|batch| *batch != 0))
}

/// Native commit and registry refresh are separate outcomes. A refresh failure cannot turn
/// an already published file or completed native rename into a pre-commit native failure.
/// The installed adapter records a refresh failure before returning it to its host.
pub struct TransferNamespaceChangeResult<T> {
    pub operation: Result<T, FsError>,
    pub refresh: Result<(), FsError>,
}

/// Run one synchronous name-changing operation and finish on every ordinary result,
/// including native failure and cancellation. Nested native helpers reborrow `change`;
/// they must not acquire a second mutation turn. An actual unwind drops the unfinished
/// transaction so the installed adapter can quarantine its retained authority.
pub fn with_transfer_namespace_change<T>(
    cancel: &Cancellation,
    operation: impl FnOnce(Option<&mut dyn TransferNamespaceChange>) -> Result<T, FsError>,
) -> TransferNamespaceChangeResult<T> {
    let mut change = match begin_transfer_namespace_change(cancel) {
        Ok(change) => change,
        Err(error) => {
            return TransferNamespaceChangeResult {
                operation: Err(error),
                refresh: Ok(()),
            };
        }
    };
    let result = match change.as_mut() {
        Some(change) => operation(Some(change.as_mut())),
        None => operation(None),
    };
    let refresh = change.map_or(Ok(()), |change| change.finish());
    TransferNamespaceChangeResult {
        operation: result,
        refresh,
    }
}

#[cfg(test)]
#[path = "transfer_namespace_tests.rs"]
mod tests;
