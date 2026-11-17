//! Transfers retain one namespace share through retirement and one retained disk-slot
//! identity. The slot consumes capacity only while the job executes native steps
//! (see `copy_operations::ready_for_io`).
use midnight_work_budget::{Gate, Permit};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(crate) const NAMESPACE_WEIGHT: u32 = crate::copy_operations::MAX_JOBS as u32;

pub(crate) fn namespace_gate() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(NAMESPACE_WEIGHT as usize))
}

pub(super) struct Leases {
    pub io: Arc<Permit>,
    pub namespace: OwnedSemaphorePermit,
}

/// The configured process-wide gate in production; isolated fixtures supply the
/// same real gate to their controller and native-call witness without global mutation.
pub(crate) struct IoBudget {
    gate: Arc<Gate>,
}

impl IoBudget {
    pub(crate) fn new(gate: Arc<Gate>) -> Arc<Self> {
        Arc::new(Self { gate })
    }
    pub(crate) fn gate(&self) -> &Arc<Gate> {
        &self.gate
    }
    #[cfg(test)]
    pub(crate) fn available_permits(&self) -> usize {
        usize::from(self.gate.snapshot().available)
    }
    /// A call-scoped consumer of the same gate (for example a checksum read).
    #[cfg(test)]
    pub(crate) fn try_acquire_owned(
        self: Arc<Self>,
    ) -> Result<Arc<Permit>, tokio::sync::TryAcquireError> {
        self.gate
            .try_acquire(midnight_work_budget::Admission::Shared)
            .map(Arc::new)
            .ok_or(tokio::sync::TryAcquireError::NoPermits)
    }
    /// FIFO call-scoped admission, queued like any other consumer of the gate.
    #[cfg(test)]
    pub(crate) async fn acquire_owned(
        self: Arc<Self>,
    ) -> Result<Arc<Permit>, tokio::sync::TryAcquireError> {
        tokio::task::spawn_blocking(move || {
            self.gate
                .acquire(midnight_work_budget::Admission::Shared, || false)
        })
        .await
        .ok()
        .flatten()
        .map(Arc::new)
        .ok_or(tokio::sync::TryAcquireError::Closed)
    }
}

#[cfg(test)]
#[path = "transfer_leases_tests.rs"]
mod tests;
