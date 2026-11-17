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

impl Registration {
    pub(super) fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        crate::copy_nested_retirement::provider_step(|| {
            let Some(owner) = Arc::into_inner(self) else {
                return Ok(());
            };
            crate::copy_nested_retirement::release_provider(owner.provider)
        })
    }
}

/// Backend-owned routing registry. Plugin calls and resource destruction stay outside its lock.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: Mutex<HashMap<ProviderId, Arc<Registration>>>,
    mounts: Arc<AtomicUsize>,
}

impl ProviderRegistry {
    pub(crate) fn unregister_on_worker(
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
        if let Some(retired) = retired {
            retired.release_on_worker()?;
        }
        Ok(true)
    }

    pub(crate) fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        use crate::copy_nested_retirement::{self as nested, Retained};
        nested::provider_step(|| {
            let Some(owner) = Arc::into_inner(self) else {
                return Ok(());
            };
            let mut owner = Retained::new(owner);
            let entries = std::mem::take(
                owner
                    .get_mut()
                    .providers
                    .get_mut()
                    .unwrap_or_else(|error| error.into_inner()),
            );
            let mut entries = Retained::new(entries.into_values());
            while let Some(entry) = entries.get_mut().next() {
                entry.lifetime.cancel();
                entry.release_on_worker()?;
            }
            drop(entries.take());
            if !nested::checked_step(|| drop(owner.take())) {
                return Err(nested::provider_error());
            }
            Ok(())
        })
    }
    /// Shared with native-owner retirement admission; keep the pane policy in one place.
    pub(crate) const MAX_MOUNTS: usize = MAX_MOUNTS;
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
        let mut construction = crate::copy_nested_retirement::Retained::new(Resolution {
            registration: Some(registration),
            permit: None,
            opened: None,
            mounted: None,
        });
        let result = (|| {
            check_cancel(&request)?;
            construction.get_mut().permit = Some(MountPermit::acquire(self.mounts.clone())?);
            let opened = construction
                .get()
                .registration
                .as_ref()
                .expect("registered resolution")
                .provider
                .resolve(address, &request)?;
            construction.get_mut().opened = Some(opened);
            check_cancel(&request)?;
            let registration = construction
                .get()
                .registration
                .as_ref()
                .expect("registered resolution");
            let reference = MountRef {
                instance: registration.instance,
                mount: next_id(&NEXT_ID)?,
                generation: next_id(&NEXT_ID)?,
            };
            // Request cancellation must not become the mount's lifetime.
            let lifetime = registration.lifetime.child();
            let state = construction.get_mut();
            state.mounted = Some(MountedLocation::new(
                address.clone(),
                reference,
                state.opened.take().expect("returned mount"),
                lifetime,
                state.registration.take().expect("registered resolution"),
                state.permit.take().expect("admitted mount"),
            ));
            check_cancel(&request)?;
            Ok(state.mounted.take().expect("completed mount"))
        })();
        let result = match result {
            Ok(mounted) => Ok(mounted),
            Err(original) => Err(construction_failure(
                original,
                construction.get_mut().release(),
            )),
        };
        drop(construction.take());
        result
    }
}

struct Resolution {
    opened: Option<super::OpenedLocation>,
    mounted: Option<MountedLocation>,
    registration: Option<Arc<Registration>>,
    permit: Option<MountPermit>,
}
impl Resolution {
    fn release(&mut self) -> Result<(), ProviderError> {
        use crate::copy_nested_retirement::{self as nested, Retained};
        let mut tail = Retained::new((self.registration.take(), self.permit.take()));
        if let Some(mounted) = self.mounted.take() {
            mounted.release_on_worker()?;
        }
        if let Some(opened) = self.opened.take() {
            nested::release_mount(opened.mount)?;
        }
        let (registration, permit) = tail.take();
        let mut permit = Retained::new(permit);
        if let Some(registration) = registration {
            registration.release_on_worker()?;
        }
        drop(permit.take());
        Ok(())
    }
}

/// The Copy context retains a separate exact teardown diagnostic. A standalone
/// caller has only this return channel, so preserve both descriptions there.
pub(super) fn construction_failure(
    original: ProviderError,
    cleanup: Result<(), ProviderError>,
) -> ProviderError {
    match cleanup {
        Ok(()) => original,
        Err(cleanup) if crate::copy_nested_retirement::current().is_some() => {
            if let Some(context) = crate::copy_nested_retirement::current() {
                context.record(crate::copy_nested_retirement::fs_error(cleanup));
            }
            original
        }
        Err(cleanup) => {
            ProviderError::Failed(format!("{original} Cleanup was not confirmed: {cleanup}"))
        }
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
#[path = "provider_construction_tests.rs"]
mod construction_tests;

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
