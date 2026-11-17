use super::*;
use crate::copy_nested_retirement as nested;
use midnight_core::{ArchiveMount, ArchiveNode, FsError, ProviderId};
use midnight_plugin_host::{CapturedFile, ContentSource};
use std::{
    num::NonZeroU32,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Debug, Default)]
struct Calls {
    tainted: AtomicBool,
    observations: AtomicUsize,
    releases: AtomicUsize,
    drops: AtomicUsize,
}
#[derive(Debug)]
struct Leaf(Arc<Calls>);
impl Drop for Leaf {
    fn drop(&mut self) {
        self.0.drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl ArchiveMount for Leaf {
    fn cleanup_unconfirmed(&self) -> bool {
        self.0.observations.fetch_add(1, Ordering::SeqCst);
        self.0.tainted.load(Ordering::Acquire)
    }
    fn id(&self) -> u64 {
        7
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        Ok(Vec::new())
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        Ok(Vec::new())
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        self.0.releases.fetch_add(1, Ordering::SeqCst);
        if self.0.tainted.load(Ordering::Acquire) {
            std::mem::forget(self);
            return Err(nested::unconfirmed());
        }
        drop(Arc::into_inner(self));
        Ok(())
    }
}

fn leased(source: Arc<dyn ArchiveMount>) -> Arc<dyn ArchiveMount> {
    MountedTarget::archive_for_taint_test(source)
}
fn navigation(source: Arc<dyn ArchiveMount>) -> Arc<dyn ArchiveMount> {
    crate::tree_actor::Session::archive_for_taint_test(source)
}

#[test]
fn each_shared_archive_wrapper_rejects_prior_taint_without_releasing_its_child() {
    for kind in 0..3 {
        let calls = Arc::new(Calls::default());
        let leaf: Arc<dyn ArchiveMount> = Arc::new(Leaf(calls.clone()));
        let (mount, domain) = match kind {
            0 => {
                let (mount, domain, _) = crate::archives::Retirement::copy_mount_for_test(leaf);
                (mount, Some(domain))
            }
            1 => (leased(leaf), None),
            _ => (navigation(leaf), None),
        };
        let other_tab = mount.clone();
        calls.tainted.store(true, Ordering::Release);
        let context = nested::Context::default();
        assert!(context.with(|| nested::release_archive(mount)).is_err());
        assert!(context.is_quarantined());
        assert!(other_tab.cleanup_unconfirmed());
        assert_eq!(calls.releases.load(Ordering::SeqCst), 0);
        assert_eq!(calls.drops.load(Ordering::SeqCst), 0);
        drop(other_tab);
        drop(domain);
        assert_eq!(calls.drops.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn clean_shared_wrappers_do_not_release_children_and_last_owner_closes_once() {
    for kind in 0..3 {
        let calls = Arc::new(Calls::default());
        let leaf: Arc<dyn ArchiveMount> = Arc::new(Leaf(calls.clone()));
        let (mount, domain) = match kind {
            0 => {
                let (mount, domain, _) = crate::archives::Retirement::copy_mount_for_test(leaf);
                (mount, Some(domain))
            }
            1 => (leased(leaf), None),
            _ => (navigation(leaf), None),
        };
        let other_tab = mount.clone();
        let context = nested::Context::default();
        context.with(|| nested::release_archive(mount)).unwrap();
        assert!(!context.is_quarantined());
        assert_eq!(calls.releases.load(Ordering::SeqCst), 0);
        context.with(|| nested::release_archive(other_tab)).unwrap();
        assert_eq!(calls.releases.load(Ordering::SeqCst), 1);
        assert_eq!(calls.drops.load(Ordering::SeqCst), 1);
        drop(domain);
    }
}

#[test]
fn deep_known_wrapper_chain_observes_one_backing_status_without_release_probes() {
    let calls = Arc::new(Calls::default());
    let leaf = Arc::new(Leaf(calls.clone()));
    let mut mount: Arc<dyn ArchiveMount> = leaf.clone();
    for _ in 0..64 {
        mount = navigation(leased(mount));
    }
    calls.tainted.store(true, Ordering::Release);
    let context = nested::Context::default();
    let references = Arc::strong_count(&leaf);
    context.with(|| nested::observe_archive(&*mount));
    assert!(context.is_quarantined());
    assert_eq!(calls.observations.load(Ordering::SeqCst), 1);
    assert_eq!(calls.releases.load(Ordering::SeqCst), 0);
    assert_eq!(Arc::strong_count(&leaf), references);
    assert_eq!(context.take_errors().len(), 1);
    assert!(context.is_quarantined());
    assert!(context.with(|| nested::release_archive(mount)).is_err());
}

struct ContentProvider(Arc<dyn ContentSource>);
impl FilesystemProvider for ContentProvider {
    fn resolve(
        &self,
        _: &StoredLocation,
        _: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        Ok(OpenedLocation {
            mount: Arc::new(ContentMount(self.0.clone())),
            root: NodeToken::new(b"root").unwrap(),
        })
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        if let Some(owner) = Arc::into_inner(self) {
            nested::content_step(|| owner.0.release_on_worker())
                .map_err(|error| ProviderError::Failed(error.to_string()))?;
        }
        Ok(())
    }
}
struct ContentMount(Arc<dyn ContentSource>);
impl MountedFilesystem for ContentMount {
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
    fn open_content(
        &self,
        _: &NodeToken,
        _: &Cancellation,
    ) -> Result<Arc<dyn ContentSource>, ProviderError> {
        Ok(self.0.clone())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        if let Some(owner) = Arc::into_inner(self) {
            nested::content_step(|| owner.0.release_on_worker())
                .map_err(|error| ProviderError::Failed(error.to_string()))?;
        }
        Ok(())
    }
}

#[test]
fn member_native_and_leased_content_forward_prior_taint_through_real_captured_file() {
    let calls = Arc::new(Calls::default());
    let member = archive::member_for_taint_test(Arc::new(Leaf(calls.clone())));
    let captured = CapturedFile::from_source(member.clone()).unwrap();
    let native = native::content_for_taint_test(captured);
    let registry = Arc::new(ProviderRegistry::default());
    let address = StoredLocation::new(
        ProviderId::new("taint-fixture").unwrap(),
        NonZeroU32::MIN,
        Arc::from(&b"root"[..]),
    );
    registry
        .register(
            address.provider().clone(),
            Arc::new(ContentProvider(native.clone())),
        )
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let file = location
        .list(location.root(), &Cancellation::default())
        .unwrap()
        .remove(0);
    let content = location
        .open_content(&file.node, &Cancellation::default())
        .unwrap();
    let other_tab = content.clone();
    calls.tainted.store(true, Ordering::Release);
    let before = calls.observations.load(Ordering::SeqCst);
    assert!(content.cleanup_unconfirmed());
    assert_eq!(calls.observations.load(Ordering::SeqCst), before + 1);
    assert_eq!(calls.releases.load(Ordering::SeqCst), 0);
    for owner in [member, native, content] {
        let context = nested::Context::default();
        assert!(
            context
                .with(|| nested::content_step(|| owner.release_on_worker()))
                .is_err()
        );
        assert!(context.is_quarantined());
    }
    assert!(other_tab.cleanup_unconfirmed());
    assert_eq!(calls.releases.load(Ordering::SeqCst), 0);
    assert_eq!(calls.drops.load(Ordering::SeqCst), 0);
    drop(other_tab);
    location.release_on_worker().unwrap();
    registry.release_on_worker().unwrap();
}

#[test]
fn observation_outside_copy_does_not_install_or_reset_a_context() {
    let calls = Arc::new(Calls::default());
    calls.tainted.store(true, Ordering::Release);
    let source = Leaf(calls.clone());
    assert!(nested::current().is_none());
    nested::observe_archive(&source);
    assert!(nested::current().is_none());
    assert!(source.cleanup_unconfirmed());
    let context = nested::Context::default();
    context.with(|| nested::observe_archive(&source));
    assert!(context.is_quarantined());
}
