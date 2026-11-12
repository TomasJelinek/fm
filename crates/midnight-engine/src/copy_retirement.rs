//! Actor cleanup capabilities are reserved before accepting provider-bearing inputs.
pub(crate) use crate::retirement::WorkerOwned;
use crate::retirement::{RetirementDomain, RetirementLease, RetirementWaiter};

pub(crate) struct ActorRetirement {
    pub(crate) domain: RetirementDomain,
    pub(crate) intake: RetirementWaiter,
    pub(crate) service: RetirementLease,
}
#[cfg(test)]
impl ActorRetirement {
    pub(crate) fn testing() -> Self {
        let domain = RetirementDomain::try_start(34, 1).unwrap();
        Self {
            intake: domain.new_waiter().unwrap(),
            service: domain.try_reserve().unwrap(),
            domain,
        }
    }
}
