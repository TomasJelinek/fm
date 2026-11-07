use super::{FilesystemProvider, ProviderError, lease::MountedLocation};
use midnight_core::{Cancellation, MountRef, ProviderId, ProviderInstanceId, StoredLocation};
use std::{
    collections::HashMap,
    num::NonZeroU64,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

// Unique across registries in this process. Never serialize or reuse these values.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
const MAX_PROVIDERS: usize = 128;
const MAX_MOUNTS: usize = 64;

pub(super) struct Registration {
    instance: ProviderInstanceId,
    provider: Arc<dyn FilesystemProvider>,
    lifetime: Cancellation,
}

/// Backend-owned routing registry. Plugin calls and resource destruction stay outside its lock.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: Mutex<HashMap<ProviderId, Arc<Registration>>>,
    mounts: Arc<AtomicUsize>,
}

impl ProviderRegistry {
    pub fn register(
        &self,
        id: ProviderId,
        provider: Arc<dyn FilesystemProvider>,
    ) -> Result<ProviderInstanceId, ProviderError> {
        let mut providers = self.providers.lock().map_err(|_| poisoned())?;
        if providers.contains_key(&id) {
            return Err(ProviderError::InvalidResponse(
                "duplicate provider registration",
            ));
        }
        if providers.len() >= MAX_PROVIDERS {
            return Err(ProviderError::ResourceLimited);
        }
        let instance = ProviderInstanceId(next_id(&NEXT_ID)?);
        providers.insert(
            id,
            Arc::new(Registration {
                instance,
                provider,
                lifetime: Cancellation::default(),
            }),
        );
        Ok(instance)
    }

    /// Revoke exactly this registration. A delayed unload cannot revoke its replacement.
    pub fn unregister(
        &self,
        id: &ProviderId,
        instance: ProviderInstanceId,
    ) -> Result<bool, ProviderError> {
        let retired = {
            let mut providers = self.providers.lock().map_err(|_| poisoned())?;
            if !providers
                .get(id)
                .is_some_and(|entry| entry.instance == instance)
            {
                return Ok(false);
            }
            let removed = providers.remove(id);
            if let Some(removed) = &removed {
                removed.lifetime.cancel();
            }
            removed
        };
        // This call is backend-only. Provider Drop may close OS/plugin resources.
        drop(retired);
        Ok(true)
    }

    pub fn resolve(
        &self,
        address: &StoredLocation,
        cancel: &Cancellation,
    ) -> Result<MountedLocation, ProviderError> {
        let registration = {
            let providers = self.providers.lock().map_err(|_| poisoned())?;
            providers
                .get(address.provider())
                .cloned()
                .ok_or(ProviderError::Unavailable)?
        };
        let request = registration.lifetime.linked(cancel);
        check_cancel(&request)?;
        let permit = MountPermit::acquire(self.mounts.clone())?;
        let opened = registration.provider.resolve(address, &request)?;
        check_cancel(&request)?;
        let reference = MountRef {
            instance: registration.instance,
            mount: next_id(&NEXT_ID)?,
            generation: next_id(&NEXT_ID)?,
        };
        // Request cancellation must not become the mount's lifetime.
        let mounted = MountedLocation::new(
            address.clone(),
            reference,
            opened,
            registration.lifetime.child(),
            registration,
            permit,
        );
        check_cancel(&request)?;
        Ok(mounted)
    }
}

/// Counts both in-flight resolution and mounts retained by content consumers.
pub(super) struct MountPermit(Arc<AtomicUsize>);

impl MountPermit {
    fn acquire(count: Arc<AtomicUsize>) -> Result<Self, ProviderError> {
        count
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_MOUNTS).then_some(count + 1)
            })
            .map_err(|_| ProviderError::ResourceLimited)?;
        Ok(Self(count))
    }
}

impl Drop for MountPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for ProviderRegistry {
    fn drop(&mut self) {
        let providers = self
            .providers
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        for registration in providers.values() {
            registration.lifetime.cancel();
        }
    }
}

pub(super) fn check_cancel(cancel: &Cancellation) -> Result<(), ProviderError> {
    if cancel.is_cancelled() {
        Err(ProviderError::Cancelled)
    } else {
        Ok(())
    }
}

pub(super) fn poisoned() -> ProviderError {
    ProviderError::Failed("Filesystem registry state is unavailable.".into())
}

fn next_id(counter: &AtomicU64) -> Result<NonZeroU64, ProviderError> {
    let value = counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| ProviderError::ResourceLimited)?;
    NonZeroU64::new(value).ok_or(ProviderError::ResourceLimited)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_exhaustion_never_wraps_or_reuses_a_value() {
        let ids = AtomicU64::new(u64::MAX - 1);
        assert_eq!(next_id(&ids).unwrap().get(), u64::MAX - 1);
        assert_eq!(next_id(&ids), Err(ProviderError::ResourceLimited));
        assert_eq!(next_id(&ids), Err(ProviderError::ResourceLimited));
        assert_eq!(ids.load(Ordering::Relaxed), u64::MAX);
    }
}
