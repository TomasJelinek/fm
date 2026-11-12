//! Canonical engine-owned archive capabilities with bounded final native cleanup.
use super::*;
use crate::{
    retirement::{RetirementDomain, RetirementLease, RetirementWaiter, WorkerOwned},
    task_lifetime::TaskLifetime,
};
use std::sync::{Mutex, Weak};
use tokio::sync::OwnedSemaphorePermit;

// Preserve both panes' existing retained provider capabilities, their current
// roots not yet admitted to those registries, and replacement listings produced
// by the two native workers. External retained outputs share this explicit cap.
const RETAINED_MOUNTS: usize =
    PANES * (crate::providers::ProviderRegistry::MAX_MOUNTS + 1) + MAX_WORKERS;
pub(crate) const RETIREMENT_CAPACITY: usize = RETAINED_MOUNTS + MAX_WORKERS + 1;
pub(crate) struct Retirement {
    pub(crate) domain: RetirementDomain,
    pub(crate) intake: RetirementWaiter,
}
#[cfg(test)]
impl Retirement {
    pub(super) fn testing() -> Self {
        let domain = RetirementDomain::try_start(RETIREMENT_CAPACITY, 1).unwrap();
        Self {
            intake: domain.new_waiter().unwrap(),
            domain,
        }
    }
}

pub(super) struct Reservation {
    lease: RetirementLease,
    permit: OwnedSemaphorePermit,
}
struct Native {
    source: Arc<dyn ArchiveMount>,
    _scope: TaskLifetime,
    _permit: OwnedSemaphorePermit,
}
pub(super) struct GuardedMount(WorkerOwned<Native>);
impl std::fmt::Debug for GuardedMount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GuardedArchiveMount")
    }
}
impl GuardedMount {
    pub(super) fn source(&self) -> &Arc<dyn ArchiveMount> {
        &self.0.source
    }
}
impl ArchiveMount for GuardedMount {
    fn id(&self) -> u64 {
        self.0.source.id()
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.0.source.list(directory, cancel)
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.0.source.read(entry, offset, length, cancel)
    }
    fn read_cooperative(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, FsError> {
        self.0
            .source
            .read_cooperative(entry, offset, length, cancelled)
    }
    fn validate(&self) -> Result<(), FsError> {
        self.0.source.validate()
    }
    fn native_container_locator(&self) -> Option<&Path> {
        self.0.source.native_container_locator()
    }
    fn persistent_member_identity(
        &self,
        entry: u64,
        expected: &[u8],
    ) -> Result<Option<Vec<u8>>, FsError> {
        self.0.source.persistent_member_identity(entry, expected)
    }
}
struct Identity {
    source: Weak<dyn ArchiveMount>,
    exposed: Weak<dyn ArchiveMount>,
    owner: Weak<GuardedMount>,
}
pub(super) struct Mounts {
    domain: RetirementDomain,
    retained: Arc<Semaphore>,
    identities: Mutex<Vec<Identity>>,
}
impl Mounts {
    pub(super) fn new(domain: RetirementDomain) -> Self {
        Self {
            domain,
            retained: Arc::new(Semaphore::new(RETAINED_MOUNTS)),
            identities: Mutex::new(Vec::with_capacity(RETAINED_MOUNTS)),
        }
    }
    /// Native worker only. Reserve before opening/capturing a new session.
    pub(super) fn reserve(&self) -> Result<Reservation, FsError> {
        let permit = self.retained.clone().try_acquire_owned().map_err(|_| {
            error(
                "Archive sessions are still in use. Close an archive tab or viewer and try again.",
            )
        })?;
        let lease = self
            .domain
            .try_reserve()
            .map_err(|_| error("Archive cleanup is still busy. Try again."))?;
        Ok(Reservation { lease, permit })
    }
    /// Native worker only: a rejected foreign input is destroyed here, not on dispatch.
    pub(super) fn adopt(
        &self,
        source: Arc<dyn ArchiveMount>,
        scope: TaskLifetime,
    ) -> Result<Arc<GuardedMount>, FsError> {
        let mut identities = self.identities.lock().unwrap_or_else(|e| e.into_inner());
        let key = Arc::downgrade(&source);
        identities.retain(|entry| entry.owner.strong_count() != 0);
        for entry in identities.iter() {
            if (entry.source.ptr_eq(&key) || entry.exposed.ptr_eq(&key))
                && let Some(owner) = entry.owner.upgrade()
            {
                return Ok(owner);
            }
        }
        // Serialize lookup/adoption so concurrent aliases share one canonical Arc.
        let reserved = self.reserve()?;
        Ok(Self::install(&mut identities, source, reserved, scope))
    }
    pub(super) fn adopt_reserved(
        &self,
        source: Arc<dyn ArchiveMount>,
        reserved: Reservation,
        scope: TaskLifetime,
    ) -> Arc<GuardedMount> {
        let mut identities = self.identities.lock().unwrap_or_else(|e| e.into_inner());
        identities.retain(|entry| entry.owner.strong_count() != 0);
        Self::install(&mut identities, source, reserved, scope)
    }
    fn install(
        identities: &mut Vec<Identity>,
        source: Arc<dyn ArchiveMount>,
        reserved: Reservation,
        scope: TaskLifetime,
    ) -> Arc<GuardedMount> {
        let original = Arc::downgrade(&source);
        let owner = Arc::new(GuardedMount(reserved.lease.own(Native {
            source,
            _scope: scope,
            _permit: reserved.permit,
        })));
        let exposed: Arc<dyn ArchiveMount> = owner.clone();
        identities.push(Identity {
            source: original,
            exposed: Arc::downgrade(&exposed),
            owner: Arc::downgrade(&owner),
        });
        owner
    }
}

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;
