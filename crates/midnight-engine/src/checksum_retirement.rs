//! Startup capabilities and the shared checksum service owner.
use crate::retirement::{RetirementDomain, RetirementLease, RetirementWaiter, WorkerOwned};
use midnight_core::{MAX_CHECKSUM_JOBS, TransferService};
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

#[derive(Clone)]
pub(crate) struct Service(Arc<WorkerOwned<Option<Arc<dyn TransferService>>>>);
impl Service {
    pub(crate) fn new(lease: RetirementLease, service: Option<Arc<dyn TransferService>>) -> Self {
        Self(Arc::new(lease.own(service)))
    }
    pub(crate) fn available(&self) -> bool {
        self.0.is_some()
    }
    pub(crate) fn provider(&self) -> &dyn TransferService {
        self.0
            .as_ref()
            .as_ref()
            .expect("admitted checksum service")
            .as_ref()
    }
}
