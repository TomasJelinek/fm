//! Transfers retain one actual disk slot and one namespace share through retirement.
use midnight_core::FsError;
use midnight_work_budget::{Admission, Gate, Permit};
use std::{io, path::Path, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

pub(crate) const NAMESPACE_WEIGHT: u32 = crate::copy_operations::MAX_JOBS as u32;

pub(crate) fn namespace_gate() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(NAMESPACE_WEIGHT as usize))
}

pub(super) struct Leases {
    pub io: Arc<Permit>,
    pub namespace: OwnedSemaphorePermit,
}

/// No permit is retained on a failed pair attempt. A legacy namespace owner may
/// need disk capacity itself, so awaiting namespace while holding I/O can deadlock.
pub(super) fn try_pair(
    disk: &Arc<Gate>,
    namespace: &Arc<Semaphore>,
) -> Result<Option<Leases>, FsError> {
    let Some(io) = disk.try_acquire(Admission::Shared) else {
        return Ok(None);
    };
    match namespace.clone().try_acquire_owned() {
        Ok(namespace) => Ok(Some(Leases {
            io: Arc::new(io),
            namespace,
        })),
        Err(TryAcquireError::NoPermits) => {
            drop(io);
            Ok(None)
        }
        Err(TryAcquireError::Closed) => {
            drop(io);
            Err(FsError::from_io(
                "acquire transfer namespace",
                Path::new(""),
                io::Error::new(io::ErrorKind::BrokenPipe, "Transfer namespace is closing."),
            ))
        }
    }
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
    #[cfg(test)]
    pub(crate) fn try_acquire_owned(self: Arc<Self>) -> Result<Arc<Permit>, TryAcquireError> {
        self.gate
            .try_acquire(Admission::Shared)
            .map(Arc::new)
            .ok_or(TryAcquireError::NoPermits)
    }
    #[cfg(test)]
    pub(crate) async fn acquire_owned(self: Arc<Self>) -> Result<Arc<Permit>, TryAcquireError> {
        loop {
            if let Some(permit) = self.gate.try_acquire(Admission::Shared) {
                return Ok(Arc::new(permit));
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}

#[cfg(test)]
#[path = "transfer_leases_tests.rs"]
mod tests;
