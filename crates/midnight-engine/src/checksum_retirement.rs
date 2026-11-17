//! Startup capabilities and the shared checksum service owner.
use super::super::retirement::{RetirementDomain, RetirementLease, RetirementWaiter, WorkerOwned};
use super::worker::{Storage, UNCLASSIFIED};
use midnight_core::{FileSystem, MAX_CHECKSUM_JOBS, TransferService};
use std::sync::Arc;

pub(crate) const MAX_SAVES: usize = 2;
pub(crate) const RETIREMENT_CAPACITY: usize = MAX_CHECKSUM_JOBS + MAX_SAVES + 2;

pub(crate) struct Retirement {
    pub(crate) domain: RetirementDomain,
    pub(crate) intake: RetirementWaiter,
    pub(crate) service: RetirementLease,
}
#[cfg(test)]
impl Retirement {
    pub(crate) fn testing() -> Self {
        let domain = RetirementDomain::try_start(RETIREMENT_CAPACITY, 1).unwrap();
        Self {
            intake: domain.new_waiter().unwrap(),
            service: domain.try_reserve().unwrap(),
            domain,
        }
    }
}

struct Providers {
    transfers: Option<Arc<dyn TransferService>>,
    /// Classifies source storage for disk admission; without it every read is
    /// exclusive (the policy for unknown storage).
    storage: Option<Arc<dyn FileSystem>>,
}

#[derive(Clone)]
pub(crate) struct Service(Arc<WorkerOwned<Providers>>);
impl Service {
    pub(crate) fn new(
        lease: RetirementLease,
        service: Option<Arc<dyn TransferService>>,
        storage: Option<Arc<dyn FileSystem>>,
    ) -> Self {
        Self(Arc::new(lease.own(Providers {
            transfers: service,
            storage,
        })))
    }
    pub(crate) fn available(&self) -> bool {
        self.0.transfers.is_some()
    }
    pub(crate) fn provider(&self) -> &dyn TransferService {
        self.0
            .transfers
            .as_deref()
            .expect("admitted checksum service")
    }
    pub(super) fn storage(&self) -> &dyn Storage {
        match &self.0.storage {
            Some(storage) => storage,
            None => &UNCLASSIFIED,
        }
    }
}
