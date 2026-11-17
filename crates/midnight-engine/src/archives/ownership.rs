//! Canonical engine-owned archive capabilities with bounded final native cleanup.
use super::*;
use crate::copy_nested_retirement::{self as nested, Retained};
use crate::retirement::{RetirementDomain, RetirementLease, RetirementWaiter, WorkerOwned};
use std::sync::{Mutex, Weak};
use tokio::sync::OwnedSemaphorePermit;

// Preserve both panes' existing retained provider capabilities, their current
// roots not yet admitted to those registries, and replacement listings produced
// by the two native workers. External retained outputs share this explicit cap.
pub(super) const RETAINED_MOUNTS: usize =
    PANES * (crate::providers::ProviderRegistry::MAX_MOUNTS + 1) + MAX_WORKERS;
// Plus the accepted navigations (MAX_REQUESTS: one per worker and the newest
// navigation of each pane waiting for a worker) and the serial intake: a new input
// waiting for its share, or stale input it retires.
pub(crate) const RETIREMENT_CAPACITY: usize = RETAINED_MOUNTS + MAX_REQUESTS + 1;
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
    source: Option<Arc<dyn ArchiveMount>>,
    permit: Option<OwnedSemaphorePermit>,
}
pub(super) struct GuardedMount(Option<WorkerOwned<Native>>);
impl Drop for GuardedMount {
    fn drop(&mut self) {
        if let Some(owner) = self.0.take() {
            if nested::current().is_some() {
                let _ = release_native(owner);
            } else {
                drop(owner);
            }
        }
    }
}
impl std::fmt::Debug for GuardedMount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GuardedArchiveMount")
    }
}
impl GuardedMount {
    pub(super) fn source(&self) -> &Arc<dyn ArchiveMount> {
        self.0
            .as_ref()
            .expect("guarded archive owner")
            .source
            .as_ref()
            .expect("archive source remains owned")
    }
}
impl ArchiveMount for GuardedMount {
    fn cleanup_unconfirmed(&self) -> bool {
        self.source().cleanup_unconfirmed()
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        let mut reference = Retained::new(self);
        if reference.get().cleanup_unconfirmed() {
            // The tainted session stays quarantined, yet only its host can end the
            // worker and return the archive process permit. The last wrapper hands
            // its reference over; a shared wrapper reference stays retained.
            match Arc::try_unwrap(reference.take()) {
                Ok(mut mount) => {
                    if let Some(owner) = mount.0.take() {
                        release_tainted(owner);
                    }
                }
                Err(shared) => std::mem::forget(shared),
            }
            return Err(nested::unconfirmed());
        }
        let Some(mut mount) = Arc::into_inner(reference.take()) else {
            return Ok(());
        };
        match mount.0.take() {
            Some(owner) => release_native(owner),
            None => Err(nested::unconfirmed()),
        }
    }
    fn id(&self) -> u64 {
        self.source().id()
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.source().list(directory, cancel)
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.source().read(entry, offset, length, cancel)
    }
    fn read_cooperative(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, FsError> {
        self.source()
            .read_cooperative(entry, offset, length, cancelled)
    }
    fn validate(&self) -> Result<(), FsError> {
        self.source().validate()
    }
    fn native_container_locator(&self) -> Option<&Path> {
        self.source().native_container_locator()
    }
    fn persistent_member_identity(
        &self,
        entry: u64,
        expected: &[u8],
    ) -> Result<Option<Vec<u8>>, FsError> {
        self.source().persistent_member_identity(entry, expected)
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
        Ok(Self::install(&mut identities, source, reserved))
    }
    pub(super) fn adopt_reserved(
        &self,
        source: Arc<dyn ArchiveMount>,
        reserved: Reservation,
    ) -> Arc<GuardedMount> {
        let mut identities = self.identities.lock().unwrap_or_else(|e| e.into_inner());
        identities.retain(|entry| entry.owner.strong_count() != 0);
        Self::install(&mut identities, source, reserved)
    }
    fn install(
        identities: &mut Vec<Identity>,
        source: Arc<dyn ArchiveMount>,
        reserved: Reservation,
    ) -> Arc<GuardedMount> {
        let original = Arc::downgrade(&source);
        let owner = Arc::new(GuardedMount(Some(reserved.lease.own(Native {
            source: Some(source),
            permit: Some(reserved.permit),
        }))));
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
impl Retirement {
    pub(crate) fn copy_mount_for_test(
        source: Arc<dyn ArchiveMount>,
    ) -> (Arc<dyn ArchiveMount>, RetirementDomain, Arc<Semaphore>) {
        let retirement = Self::testing();
        let mounts = Mounts::new(retirement.domain.clone());
        let mount = mounts.adopt(source).unwrap();
        (mount, retirement.domain, mounts.retained.clone())
    }
}

/// Give a tainted session's host reference its final release. The host stops the
/// worker and returns its process permit but keeps the tainted source; this
/// wrapper's retained-mount slot and cleanup lease stay quarantined as well.
fn release_tainted(owner: WorkerOwned<Native>) {
    let context = nested::current().unwrap_or_default();
    let (native, lease) = owner.into_parts();
    let mut native = Retained::new(native);
    let _lease = Retained::new(lease);
    let Some(source) = native.get_mut().source.take() else {
        return;
    };
    // Deliberately not a checked step: a quarantined context must still reach the
    // host, whose tainted release performs no source teardown.
    let released =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source.release_on_worker()));
    match released {
        Ok(Ok(())) => {}
        Ok(Err(error)) => context.record(error),
        Err(payload) => {
            std::mem::forget(payload);
            context.record(nested::unconfirmed());
        }
    }
}

fn release_native(owner: WorkerOwned<Native>) -> Result<(), FsError> {
    let context = nested::current().unwrap_or_default();
    let (native, lease) = owner.into_parts();
    let mut native = Retained::new(native);
    let mut lease = Retained::new(lease);
    let result = context.with(|| {
        if context.is_quarantined() {
            return Err(nested::unconfirmed());
        }
        let source = native
            .get_mut()
            .source
            .take()
            .ok_or_else(nested::unconfirmed)?;
        nested::release_archive(source)?;
        if !nested::checked_step(|| drop(native.get_mut().permit.take())) {
            return Err(nested::unconfirmed());
        }
        if !nested::checked_step(|| drop(native.take())) {
            return Err(nested::unconfirmed());
        }
        drop(lease.take());
        Ok(())
    });
    if let Err(error) = &result {
        context.record(error.clone());
    }
    result
}

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;
