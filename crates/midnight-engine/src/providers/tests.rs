use super::*;
use midnight_core::ProviderId;
use std::num::NonZeroU32;

struct EmptyProvider;
struct EmptyMount;

impl FilesystemProvider for EmptyProvider {
    fn resolve(
        &self,
        _address: &StoredLocation,
        _cancel: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        Ok(OpenedLocation {
            mount: Arc::new(EmptyMount),
            root: NodeToken::new(b"root").unwrap(),
        })
    }
}
impl MountedFilesystem for EmptyMount {
    fn list(
        &self,
        _directory: &NodeToken,
        _cancel: &Cancellation,
        _emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        Ok(())
    }
    fn parent(
        &self,
        _node: &NodeToken,
        _cancel: &Cancellation,
    ) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
}

fn address(id: &str) -> StoredLocation {
    StoredLocation::new(
        ProviderId::new(id).unwrap(),
        NonZeroU32::new(1).unwrap(),
        Arc::from(&b"raw address"[..]),
    )
}

#[test]
fn registered_provider_resolves_and_missing_provider_stays_unavailable() {
    let registry = ProviderRegistry::default();
    let address = address("test.one");
    assert!(matches!(
        registry.resolve(&address, &Cancellation::default()),
        Err(ProviderError::Unavailable)
    ));
    registry
        .register(address.provider().clone(), Arc::new(EmptyProvider))
        .unwrap();
    assert!(registry.resolve(&address, &Cancellation::default()).is_ok());
}

#[test]
fn duplicate_registration_never_replaces_a_live_provider() {
    let registry = ProviderRegistry::default();
    let address = address("test.one");
    let first = registry
        .register(address.provider().clone(), Arc::new(EmptyProvider))
        .unwrap();
    assert!(
        registry
            .register(address.provider().clone(), Arc::new(EmptyProvider))
            .is_err()
    );
    let mount = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    assert_eq!(mount.root().mount.instance, first);
    assert_eq!(mount.address(), &address);
    assert_eq!(
        mount
            .parent(mount.root(), &Cancellation::default())
            .unwrap(),
        None
    );
}

#[test]
fn identical_node_tokens_cannot_cross_mounts_or_registries() {
    let first_registry = ProviderRegistry::default();
    let second_registry = ProviderRegistry::default();
    let address = address("test.one");
    first_registry
        .register(address.provider().clone(), Arc::new(EmptyProvider))
        .unwrap();
    second_registry
        .register(address.provider().clone(), Arc::new(EmptyProvider))
        .unwrap();
    let first = first_registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let second = second_registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let another = first_registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    assert_eq!(first.root().token, second.root().token);
    for foreign in [second.root(), another.root()] {
        assert!(matches!(
            first.list(foreign, &Cancellation::default()),
            Err(ProviderError::StaleReference)
        ));
    }
}

#[test]
fn old_registration_cannot_revoke_replacement_and_cancelled_resolve_does_not_cancel_mount() {
    let registry = ProviderRegistry::default();
    let address = address("test.one");
    let first_id = registry
        .register(address.provider().clone(), Arc::new(EmptyProvider))
        .unwrap();
    let request = Cancellation::default();
    let old = registry.resolve(&address, &request).unwrap();
    request.cancel();
    assert!(old.list(old.root(), &Cancellation::default()).is_ok());
    assert!(registry.unregister(address.provider(), first_id).unwrap());
    let new_id = registry
        .register(address.provider().clone(), Arc::new(EmptyProvider))
        .unwrap();
    assert_ne!(first_id, new_id);
    assert!(!registry.unregister(address.provider(), first_id).unwrap());
    assert!(matches!(
        old.list(old.root(), &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
    assert!(registry.resolve(&address, &Cancellation::default()).is_ok());
    assert!(matches!(
        registry.resolve(&address, &request),
        Err(ProviderError::Cancelled)
    ));
}

struct FilesProvider {
    duplicate: bool,
    short_read: bool,
}
struct FilesMount {
    duplicate: bool,
    short_read: bool,
}

impl FilesystemProvider for FilesProvider {
    fn resolve(
        &self,
        _address: &StoredLocation,
        _cancel: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        Ok(OpenedLocation {
            mount: Arc::new(FilesMount {
                duplicate: self.duplicate,
                short_read: self.short_read,
            }),
            root: token(1),
        })
    }
}
fn token(value: u8) -> NodeToken {
    NodeToken::new(&[value]).unwrap()
}
fn file(value: u8) -> ProviderEntry {
    ProviderEntry {
        token: token(value),
        raw_name: Arc::from([0xff, value]),
        display_name: "same caption".into(),
        kind: EntryKind::File,
        metadata: EntryMetadata {
            size: Some(5),
            ..EntryMetadata::default()
        },
    }
}
impl MountedFilesystem for FilesMount {
    fn list(
        &self,
        _directory: &NodeToken,
        _cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        if emit(file(2)) {
            let _ = emit(file(if self.duplicate { 2 } else { 3 }));
        }
        Ok(())
    }
    fn parent(
        &self,
        node: &NodeToken,
        _cancel: &Cancellation,
    ) -> Result<Option<NodeToken>, ProviderError> {
        Ok((node != &token(1)).then(|| token(1)))
    }
    fn open_content(
        &self,
        _node: &NodeToken,
        _cancel: &Cancellation,
    ) -> Result<Arc<dyn ContentSource>, ProviderError> {
        Ok(Arc::new(TestContent {
            short_read: self.short_read,
        }))
    }
}

#[derive(Debug)]
struct TestContent {
    short_read: bool,
}
impl ContentSource for TestContent {
    fn length(&self) -> u64 {
        5
    }
    fn display_name(&self) -> String {
        "same caption".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), midnight_plugin_host::Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, midnight_plugin_host::Error> {
        let data = b"hello";
        let begin = usize::try_from(offset).unwrap().min(data.len());
        let end = begin.saturating_add(length as usize).min(data.len());
        let end = if self.short_read { begin } else { end };
        Ok(data[begin..end].to_vec())
    }
}

fn files(
    duplicate: bool,
    short_read: bool,
) -> (
    ProviderRegistry,
    MountedLocation,
    midnight_core::ProviderInstanceId,
) {
    let registry = ProviderRegistry::default();
    let address = address("test.files");
    let instance = registry
        .register(
            address.provider().clone(),
            Arc::new(FilesProvider {
                duplicate,
                short_read,
            }),
        )
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    (registry, location, instance)
}

#[test]
fn raw_names_stay_distinct_and_parent_is_provider_owned() {
    let (_registry, mount, _) = files(false, false);
    let rows = mount.list(mount.root(), &Cancellation::default()).unwrap();
    assert_eq!(rows[0].entry.display_name, rows[1].entry.display_name);
    assert_ne!(rows[0].entry.raw_name, rows[1].entry.raw_name);
    assert_ne!(rows[0].node, rows[1].node);
    assert_eq!(
        mount
            .parent(&rows[0].node, &Cancellation::default())
            .unwrap()
            .as_ref(),
        Some(mount.root())
    );
    assert!(matches!(
        mount.list(&rows[0].node, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
}

#[test]
fn failed_listing_does_not_issue_partial_node_references() {
    let (_registry, mount, _) = files(true, false);
    assert!(matches!(
        mount.list(mount.root(), &Cancellation::default()),
        Err(ProviderError::InvalidResponse(_))
    ));
    let forged = midnight_core::NodeRef {
        mount: mount.root().mount,
        token: token(2),
    };
    assert!(matches!(
        mount.open_content(&forged, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
}

#[test]
fn viewer_broker_keeps_content_after_location_close_and_rejects_revocation() {
    let (registry, mount, instance) = files(false, false);
    let address = mount.address().clone();
    let rows = mount.list(mount.root(), &Cancellation::default()).unwrap();
    let source = mount
        .open_content(&rows[0].node, &Cancellation::default())
        .unwrap();
    let captured = midnight_plugin_host::CapturedFile::from_source(source).unwrap();
    assert!(captured.path().as_os_str().is_empty());
    drop(mount);
    assert_eq!(captured.read_at(0, 256).unwrap(), b"hello");
    assert_eq!(captured.read_at(4, 8).unwrap(), b"o");
    registry.unregister(address.provider(), instance).unwrap();
    assert!(captured.read_at(0, 5).is_err());
}

#[test]
fn viewer_broker_rejects_short_provider_reads() {
    let (_registry, mount, _) = files(false, true);
    let rows = mount.list(mount.root(), &Cancellation::default()).unwrap();
    let source = mount
        .open_content(&rows[0].node, &Cancellation::default())
        .unwrap();
    let captured = midnight_plugin_host::CapturedFile::from_source(source).unwrap();
    assert!(captured.read_at(0, 5).is_err());
}

#[test]
fn returned_content_source_also_enforces_the_shared_broker_read_contract() {
    let (_registry, mount, _) = files(false, false);
    let rows = mount.list(mount.root(), &Cancellation::default()).unwrap();
    let source = mount
        .open_content(&rows[0].node, &Cancellation::default())
        .unwrap();
    assert!(source.read_at(0, 0).is_err());
    assert!(source.read_at(0, 256 * 1024 + 1).is_err());
    assert!(source.read_at(u64::MAX, 1).is_err());
    assert_eq!(source.read_at(4, 256).unwrap(), b"o");
    let (_registry, mount, _) = files(false, true);
    let rows = mount.list(mount.root(), &Cancellation::default()).unwrap();
    let source = mount
        .open_content(&rows[0].node, &Cancellation::default())
        .unwrap();
    assert!(source.read_at(0, 5).is_err());
}

#[test]
fn dropping_registry_revokes_retained_mount() {
    let (registry, mount, _) = files(false, false);
    drop(registry);
    assert!(matches!(
        mount.list(mount.root(), &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
}

#[test]
fn blocked_resolution_does_not_hold_registry_lock_and_late_output_is_rejected() {
    use std::{
        sync::{Mutex, mpsc},
        time::Duration,
    };
    struct BlockingProvider {
        entered: mpsc::Sender<()>,
        resume: Mutex<mpsc::Receiver<()>>,
    }
    impl FilesystemProvider for BlockingProvider {
        fn resolve(
            &self,
            _address: &StoredLocation,
            _cancel: &Cancellation,
        ) -> Result<OpenedLocation, ProviderError> {
            self.entered.send(()).unwrap();
            self.resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            Ok(OpenedLocation {
                mount: Arc::new(EmptyMount),
                root: token(1),
            })
        }
    }
    let registry = Arc::new(ProviderRegistry::default());
    let slow_address = address("test.slow");
    let fast_address = address("test.fast");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let instance = registry
        .register(
            slow_address.provider().clone(),
            Arc::new(BlockingProvider {
                entered: entered_tx,
                resume: Mutex::new(resume_rx),
            }),
        )
        .unwrap();
    registry
        .register(fast_address.provider().clone(), Arc::new(EmptyProvider))
        .unwrap();
    let slow = {
        let registry = registry.clone();
        let address = slow_address.clone();
        std::thread::spawn(move || registry.resolve(&address, &Cancellation::default()))
    };
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (fast_tx, fast_rx) = mpsc::channel();
    let fast = {
        let registry = registry.clone();
        std::thread::spawn(move || {
            let result = registry.resolve(&fast_address, &Cancellation::default());
            let _ = fast_tx.send(result.is_ok());
        })
    };
    let fast_completed = fast_rx.recv_timeout(Duration::from_secs(2));
    // Release the worker before asserting, so a regression cannot strand the test.
    if fast_completed == Ok(true) {
        registry
            .unregister(slow_address.provider(), instance)
            .unwrap();
    }
    resume_tx.send(()).unwrap();
    let slow_result = slow.join().unwrap();
    fast.join().unwrap();
    assert_eq!(fast_completed, Ok(true));
    assert!(matches!(slow_result, Err(ProviderError::Cancelled)));
}
