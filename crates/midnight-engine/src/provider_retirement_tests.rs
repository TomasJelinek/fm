use super::*;
use crate::copy_nested_retirement as retirement;
use std::{
    num::NonZeroU32,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

#[derive(Default)]
struct Calls {
    order: Mutex<Vec<&'static str>>,
    drops: AtomicUsize,
}
struct Mount(Arc<Calls>);
impl MountedFilesystem for Mount {
    fn list(
        &self,
        _: &NodeToken,
        _: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        emit(ProviderEntry {
            token: NodeToken::new(b"file").unwrap(),
            raw_name: Arc::from(&b"file"[..]),
            display_name: "file".into(),
            kind: EntryKind::File,
            metadata: EntryMetadata::default(),
        });
        Ok(())
    }
    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
    fn resolve_target(
        &self,
        _: &NodeToken,
        _: &Cancellation,
    ) -> Result<ProviderTarget, ProviderError> {
        Ok(ProviderTarget::Native(std::path::PathBuf::from(
            "metadata-only-fixture",
        )))
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        if let Some(owner) = Arc::into_inner(self) {
            owner.0.order.lock().unwrap().push("mount");
        }
        Ok(())
    }
}
struct Provider {
    calls: Arc<Calls>,
    hold: Option<(mpsc::Sender<()>, Mutex<mpsc::Receiver<()>>)>,
}
impl FilesystemProvider for Provider {
    fn resolve(
        &self,
        _: &StoredLocation,
        _: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        Ok(OpenedLocation {
            root: NodeToken::new(b"root").unwrap(),
            mount: Arc::new(Mount(self.calls.clone())),
        })
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        if let Some(owner) = Arc::into_inner(self) {
            if let Some((entered, release)) = &owner.hold {
                entered.send(()).unwrap();
                release.lock().unwrap().recv().unwrap();
            }
            owner.calls.order.lock().unwrap().push("provider");
            owner.calls.drops.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}
fn provider(calls: &Arc<Calls>) -> Arc<dyn FilesystemProvider> {
    Arc::new(Provider {
        calls: calls.clone(),
        hold: None,
    })
}
fn address() -> StoredLocation {
    StoredLocation::new(
        midnight_core::ProviderId::new("retirement.test").unwrap(),
        NonZeroU32::MIN,
        Arc::from(&b"root"[..]),
    )
}
struct Release(Option<mpsc::Sender<()>>);
impl Release {
    fn now(&mut self) {
        if let Some(send) = self.0.take() {
            let _ = send.send(());
        }
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        self.now();
    }
}

#[test]
fn checked_unregister_drains_outside_mutex_and_preserves_replacement_instance() {
    let registry = Arc::new(ProviderRegistry::default());
    let address = address();
    let calls = Arc::new(Calls::default());
    let (entered, observe) = mpsc::channel();
    let (send, wait) = mpsc::channel();
    let mut release = Release(Some(send));
    let first = registry
        .register(
            address.provider().clone(),
            Arc::new(Provider {
                calls: calls.clone(),
                hold: Some((entered, Mutex::new(wait))),
            }),
        )
        .unwrap();
    let worker_registry = registry.clone();
    let id = address.provider().clone();
    let worker = thread::spawn(move || worker_registry.unregister_on_worker(&id, first));
    let entered = observe.recv_timeout(Duration::from_secs(5));
    let (replaced, replacement) = mpsc::channel();
    let other_registry = registry.clone();
    let id = address.provider().clone();
    let other_calls = Arc::new(Calls::default());
    let replacement_calls = other_calls.clone();
    let replacing = thread::spawn(move || {
        replaced
            .send(other_registry.register(id, provider(&replacement_calls)))
            .unwrap();
    });
    let replacement = replacement.recv_timeout(Duration::from_secs(5));
    release.now();
    let retired = worker.join().unwrap();
    replacing.join().unwrap();
    entered.unwrap();
    assert_eq!(retired, Ok(true));
    let replacement = replacement.unwrap().unwrap();
    assert!(
        !registry
            .unregister_on_worker(address.provider(), first)
            .unwrap()
    );
    assert!(
        registry
            .unregister_on_worker(address.provider(), replacement)
            .unwrap()
    );
    assert_eq!(calls.drops.load(Ordering::SeqCst), 1);
    assert_eq!(other_calls.drops.load(Ordering::SeqCst), 1);
    registry.release_on_worker().unwrap();
}

#[test]
fn shared_registry_release_does_not_wait_for_another_tab() {
    let registry = Arc::new(ProviderRegistry::default());
    let calls = Arc::new(Calls::default());
    registry
        .register(address().provider().clone(), provider(&calls))
        .unwrap();
    registry.clone().release_on_worker().unwrap();
    assert_eq!(calls.drops.load(Ordering::SeqCst), 0);
    registry.release_on_worker().unwrap();
    assert_eq!(calls.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn last_mounted_location_releases_mount_before_registration() {
    let registry = Arc::new(ProviderRegistry::default());
    let calls = Arc::new(Calls::default());
    let address = address();
    let instance = registry
        .register(address.provider().clone(), provider(&calls))
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    location.clone().release_on_worker().unwrap();
    assert!(calls.order.lock().unwrap().is_empty());
    registry
        .unregister_on_worker(address.provider(), instance)
        .unwrap();
    location.release_on_worker().unwrap();
    assert_eq!(*calls.order.lock().unwrap(), ["mount", "provider"]);
    registry.release_on_worker().unwrap();
}

#[test]
fn root_seed_consumes_its_actual_inner_mount_once() {
    let calls = Arc::new(Calls::default());
    let root = root_seed::wrap(
        OpenedLocation {
            root: NodeToken::new(b"root").unwrap(),
            mount: Arc::new(Mount(calls.clone())),
        },
        Vec::new(),
        root_seed::RootReadPolicy::NativeRefresh,
        &Cancellation::default(),
    )
    .unwrap();
    retirement::release_mount(root.mount).unwrap();
    assert_eq!(*calls.order.lock().unwrap(), ["mount"]);
}

#[test]
fn unknown_provider_with_other_domain_owner_cannot_acknowledge_native_close() {
    struct Native(Arc<AtomicUsize>);
    impl Drop for Native {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct Unknown {
        _owner: crate::retirement::WorkerOwned<Native>,
    }
    impl FilesystemProvider for Unknown {
        fn resolve(
            &self,
            _: &StoredLocation,
            _: &Cancellation,
        ) -> Result<OpenedLocation, ProviderError> {
            Err(ProviderError::Unsupported)
        }
    }
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let owner = domain.try_reserve().unwrap().own(Native(drops.clone()));
    let context = retirement::Context::default();
    assert!(
        context
            .with(|| retirement::release_provider(Arc::new(Unknown { _owner: owner })))
            .is_err()
    );
    assert!(context.is_quarantined());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert!(domain.try_reserve().is_err());
}

#[test]
fn shared_unknown_task_lifetime_consumes_only_its_reference() {
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let owner = Arc::new(domain.try_reserve().unwrap().own(()));
    let lifetime = crate::task_lifetime::TaskLifetime::new(owner.clone());
    let context = retirement::Context::default();
    context.with(|| drop(lifetime));
    assert!(!context.is_quarantined());
    let owner = Arc::into_inner(owner).expect("the independent reference is still owned");
    let ((), lease) = owner.into_parts();
    drop(lease);
    assert!(domain.try_reserve().is_ok());
}

#[test]
fn target_context_keeps_registration_until_its_actual_last_location_releases() {
    let registry = Arc::new(ProviderRegistry::default());
    let calls = Arc::new(Calls::default());
    let address = address();
    let instance = registry
        .register(address.provider().clone(), provider(&calls))
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let target = location
        .resolve_target(location.root(), &Cancellation::default())
        .unwrap();
    registry
        .unregister_on_worker(address.provider(), instance)
        .unwrap();
    location.release_on_worker().unwrap();
    assert!(calls.order.lock().unwrap().is_empty());
    target.release_on_worker().unwrap();
    assert_eq!(*calls.order.lock().unwrap(), ["mount", "provider"]);
    registry.release_on_worker().unwrap();
}

#[test]
fn failed_mounted_release_retains_registration_and_mount_accounting() {
    struct Refusal;
    impl MountedFilesystem for Refusal {
        fn list(
            &self,
            _: &NodeToken,
            _: &Cancellation,
            _: &mut dyn FnMut(ProviderEntry) -> bool,
        ) -> Result<(), ProviderError> {
            Ok(())
        }
        fn parent(
            &self,
            _: &NodeToken,
            _: &Cancellation,
        ) -> Result<Option<NodeToken>, ProviderError> {
            Ok(None)
        }
        fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
            std::mem::forget(self);
            Err(ProviderError::Failed("controlled close refusal".into()))
        }
    }
    struct RefusingProvider(Arc<Calls>);
    impl FilesystemProvider for RefusingProvider {
        fn resolve(
            &self,
            _: &StoredLocation,
            _: &Cancellation,
        ) -> Result<OpenedLocation, ProviderError> {
            Ok(OpenedLocation {
                root: NodeToken::new(b"root").unwrap(),
                mount: Arc::new(Refusal),
            })
        }
        fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
            if let Some(owner) = Arc::into_inner(self) {
                owner.0.drops.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }
    let registry = Arc::new(ProviderRegistry::default());
    let calls = Arc::new(Calls::default());
    let address = address();
    registry
        .register(
            address.provider().clone(),
            Arc::new(RefusingProvider(calls.clone())),
        )
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let context = retirement::Context::default();
    assert!(context.with(|| location.release_on_worker()).is_err());
    assert!(context.is_quarantined());
    let mut remaining = Vec::new();
    for _ in 1..ProviderRegistry::MAX_MOUNTS {
        remaining.push(
            registry
                .resolve(&address, &Cancellation::default())
                .unwrap(),
        );
    }
    assert!(matches!(
        registry.resolve(&address, &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
    assert_eq!(calls.drops.load(Ordering::SeqCst), 0);
    // Other independent references use ordinary non-Copy destruction. The one
    // refused location retains its own registration and accounting permanently.
    drop(remaining);
    registry.release_on_worker().unwrap();
    assert_eq!(calls.drops.load(Ordering::SeqCst), 0);
}
