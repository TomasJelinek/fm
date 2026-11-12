//! Target publication must honor admission and revocation across a blocking provider call.
use super::*;
use midnight_core::{NodeRef, ProviderId, ProviderInstanceId};
use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    time::Duration,
};

const DEADLINE: Duration = Duration::from_secs(5);

struct Owner(Arc<BlockingMount>);
struct BlockingMount {
    calls: AtomicUsize,
    entered: SyncSender<Cancellation>,
    resume: Mutex<Receiver<()>>,
}

impl FilesystemProvider for Owner {
    fn resolve(
        &self,
        _: &StoredLocation,
        _: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        Ok(OpenedLocation {
            mount: self.0.clone(),
            root: token(1),
        })
    }
}

impl MountedFilesystem for BlockingMount {
    fn list(
        &self,
        _: &NodeToken,
        _: &Cancellation,
        _: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        Ok(())
    }

    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }

    fn resolve_target(
        &self,
        _: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<ProviderTarget, ProviderError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.entered.send(cancel.clone()).unwrap();
        self.resume.lock().unwrap().recv_timeout(DEADLINE).unwrap();
        // A provider may finish successfully concurrently with cancellation.
        Ok(ProviderTarget::Native(PathBuf::from(
            "exact retained target",
        )))
    }
}

struct Fixture {
    registry: ProviderRegistry,
    address: StoredLocation,
    instance: ProviderInstanceId,
    location: MountedLocation,
    mount: Arc<BlockingMount>,
    entered: Receiver<Cancellation>,
    resume: SyncSender<()>,
}

fn token(value: u8) -> NodeToken {
    NodeToken::new(&[value]).unwrap()
}

fn fixture() -> Fixture {
    let (entered_tx, entered) = sync_channel(1);
    let (resume, resume_rx) = sync_channel(1);
    let mount = Arc::new(BlockingMount {
        calls: AtomicUsize::new(0),
        entered: entered_tx,
        resume: Mutex::new(resume_rx),
    });
    let registry = ProviderRegistry::default();
    let address = StoredLocation::new(
        ProviderId::new("test.target.contract").unwrap(),
        NonZeroU32::MIN,
        Arc::from([]),
    );
    let instance = registry
        .register(address.provider().clone(), Arc::new(Owner(mount.clone())))
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    Fixture {
        registry,
        address,
        instance,
        location,
        mount,
        entered,
        resume,
    }
}

#[test]
fn target_cancelled_while_resolving_is_discarded_without_revoking_the_mount() {
    let fixture = fixture();
    let caller = Cancellation::default();
    let request = caller.clone();
    let location = fixture.location.clone();
    let job = std::thread::spawn(move || location.resolve_target(location.root(), &request));
    let linked = fixture.entered.recv_timeout(DEADLINE).unwrap();
    assert!(!linked.is_cancelled());
    caller.cancel();
    assert!(
        linked.is_cancelled(),
        "provider must observe caller cancellation"
    );
    fixture.resume.send(()).unwrap();
    assert!(matches!(job.join().unwrap(), Err(ProviderError::Cancelled)));

    let location = fixture.location.clone();
    let retry = std::thread::spawn(move || {
        location.resolve_target(location.root(), &Cancellation::default())
    });
    let linked = fixture.entered.recv_timeout(DEADLINE).unwrap();
    assert!(
        !linked.is_cancelled(),
        "request cancellation must not revoke its mount"
    );
    fixture.resume.send(()).unwrap();
    let target = retry.join().unwrap().unwrap();
    assert!(matches!(
        target.value().unwrap(),
        ProviderTarget::Native(path) if path == Path::new("exact retained target")
    ));
    assert_eq!(fixture.mount.calls.load(Ordering::Acquire), 2);
}

#[test]
fn target_unregister_during_resolution_cancels_provider_and_rejects_success() {
    let fixture = fixture();
    let caller = Cancellation::default();
    let request = caller.clone();
    let location = fixture.location.clone();
    let job = std::thread::spawn(move || location.resolve_target(location.root(), &request));
    let linked = fixture.entered.recv_timeout(DEADLINE).unwrap();
    assert!(!linked.is_cancelled());
    assert!(
        fixture
            .registry
            .unregister(fixture.address.provider(), fixture.instance)
            .unwrap()
    );
    assert!(
        linked.is_cancelled(),
        "provider must observe registration revocation"
    );
    assert!(
        !caller.is_cancelled(),
        "registration cannot cancel the caller's other work"
    );
    fixture.resume.send(()).unwrap();
    assert!(matches!(job.join().unwrap(), Err(ProviderError::Cancelled)));
    assert!(matches!(
        fixture
            .location
            .resolve_target(fixture.location.root(), &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
    assert_eq!(fixture.mount.calls.load(Ordering::Acquire), 1);
}

#[test]
fn target_registry_drop_during_resolution_rejects_late_success() {
    let fixture = fixture();
    let location = fixture.location.clone();
    let job = std::thread::spawn(move || {
        location.resolve_target(location.root(), &Cancellation::default())
    });
    let linked = fixture.entered.recv_timeout(DEADLINE).unwrap();
    drop(fixture.registry);
    assert!(linked.is_cancelled());
    fixture.resume.send(()).unwrap();
    assert!(matches!(job.join().unwrap(), Err(ProviderError::Cancelled)));
    assert_eq!(fixture.mount.calls.load(Ordering::Acquire), 1);
}

#[test]
fn target_foreign_unissued_and_precancelled_nodes_never_reach_provider() {
    let fixture = fixture();
    let other = fixture
        .registry
        .resolve(&fixture.address, &Cancellation::default())
        .unwrap();
    assert_eq!(other.root().token, fixture.location.root().token);
    assert_ne!(other.root().mount, fixture.location.root().mount);
    let unknown = NodeRef {
        mount: fixture.location.root().mount,
        token: token(2),
    };
    for node in [other.root(), &unknown] {
        assert!(matches!(
            fixture
                .location
                .resolve_target(node, &Cancellation::default()),
            Err(ProviderError::StaleReference)
        ));
    }
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(matches!(
        fixture
            .location
            .resolve_target(fixture.location.root(), &cancelled),
        Err(ProviderError::Cancelled)
    ));
    assert_eq!(fixture.mount.calls.load(Ordering::Acquire), 0);
}
