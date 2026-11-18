use super::*;
use crate::copy_nested_retirement as nested;
use midnight_core::{
    ArchiveAddress, ArchiveLocation, ArchiveMount, ArchiveNode, FsError, ProviderId,
};
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
        // The shared release gave up its count, so the other tab is now the last owner.
        // Released in the same quarantined Copy, only the guard (kind 0) hands its
        // reference to the leaf's failing release; a wrapper gives its leaf reference to
        // the leaf's default quarantined release, which retains it. Nothing is
        // destroyed, and no destruction is queued on a cleanup worker.
        assert!(context.with(|| nested::release_archive(other_tab)).is_err());
        assert_eq!(
            calls.releases.load(Ordering::SeqCst),
            usize::from(kind == 0)
        );
        drop(domain);
        assert_eq!(calls.drops.load(Ordering::SeqCst), 0);
    }
}

/// How one owner of a shared archive session gives up its reference.
#[derive(Clone, Copy, Debug)]
enum Release {
    /// A direct consuming release outside Copy, like a tab, tree or navigation owner's.
    Worker,
    /// Inside a Copy whose retirement is already quarantined.
    Quarantined,
}
impl Release {
    const ALL: [Self; 2] = [Self::Worker, Self::Quarantined];
    fn context(self) -> nested::Context {
        let context = nested::Context::default();
        if matches!(self, Self::Quarantined) {
            context.quarantine();
        }
        context
    }
    /// True when the release failed, as every release of a tainted session does.
    fn archive(self, mount: Arc<dyn ArchiveMount>) -> bool {
        self.context()
            .with(|| nested::release_archive(mount))
            .is_err()
    }
    fn mounted(self, mount: Arc<dyn MountedFilesystem>) -> bool {
        self.context().with(|| mount.release_on_worker()).is_err()
    }
    fn content(self, content: Arc<dyn ContentSource>) -> bool {
        self.context().with(|| content.release_on_worker()).is_err()
    }
}

/// A tab's tainted guarded session over the filesystem host fake, which holds one of
/// `active`'s archive process permits until its worker stops. The probe reports the
/// host releases reached and whether the worker stopped or the session was destroyed.
fn tainted_tab(
    active: &Arc<tokio::sync::Semaphore>,
) -> (Arc<dyn ArchiveMount>, impl Fn() -> (usize, bool, bool)) {
    let (host, probe) = crate::archives::Retirement::tainted_host_for_test(active);
    let (tab, _, _) = crate::archives::Retirement::copy_mount_for_test(host);
    probe.taint();
    (tab, move || {
        (probe.releases(), probe.stopped(), probe.dropped())
    })
}

/// The second owner's release must hand the session's last reference to the host's
/// failing release exactly once: it stops the worker and returns its permit without
/// destroying the tainted source.
fn release_shared_tainted_session(
    first: impl FnOnce() -> bool,
    last: impl FnOnce() -> bool,
    host: impl Fn() -> (usize, bool, bool),
    active: &tokio::sync::Semaphore,
    case: &str,
) {
    assert!(first(), "{case}: a tainted release succeeded");
    assert_eq!(
        host(),
        (0, false, false),
        "{case}: a shared release ended it"
    );
    assert_eq!(active.available_permits(), 1, "{case}");
    assert!(last(), "{case}: a tainted release succeeded");
    assert_eq!(
        host(),
        (1, true, false),
        "{case}: the last owner missed the host"
    );
    assert_eq!(active.available_permits(), 2, "{case}: process permit kept");
}

#[test]
fn tainted_target_and_navigation_wrappers_hand_their_guard_reference_down() {
    let active = Arc::new(tokio::sync::Semaphore::new(2));
    for kind in 0..3 {
        for release in Release::ALL {
            for wrapper_first in [true, false] {
                let (tab, host) = tainted_tab(&active);
                let wrapper = match kind {
                    0 => leased(tab.clone()),
                    1 => navigation(tab.clone()),
                    _ => navigation(leased(tab.clone())),
                };
                // Another holder of the same wrapper, e.g. a Copy source, gives up only
                // its count.
                let holder = wrapper.clone();
                let case = format!("kind {kind}, {release:?}, wrapper first: {wrapper_first}");
                assert!(release.archive(holder), "{case}");
                let (first, last) = if wrapper_first {
                    (wrapper, tab)
                } else {
                    (tab, wrapper)
                };
                release_shared_tainted_session(
                    || release.archive(first),
                    || release.archive(last),
                    &host,
                    &active,
                    &case,
                );
            }
        }
    }
}

#[test]
fn tree_capture_gives_up_its_tainted_guard_reference_so_the_last_owner_stops_the_worker() {
    let active = Arc::new(tokio::sync::Semaphore::new(2));
    for release in Release::ALL {
        for tree_first in [true, false] {
            let (tab, host) = tainted_tab(&active);
            let location = ArchiveLocation {
                mount: Some(tab.clone()),
                address: ArchiveAddress {
                    provider: "taint-fixture".into(),
                    format: None,
                    container: std::env::temp_dir().join("tainted.zip"),
                    source_version: vec![1],
                    components: Vec::new(),
                },
                directory: 1,
                trail: vec![(1, "tainted.zip".into())],
            };
            let tree = captured_archive(&location, &Cancellation::default())
                .unwrap()
                .mount;
            drop(location);
            let case = format!("{release:?}, tree first: {tree_first}");
            if tree_first {
                release_shared_tainted_session(
                    || release.mounted(tree),
                    || release.archive(tab),
                    &host,
                    &active,
                    &case,
                );
            } else {
                release_shared_tainted_session(
                    || release.archive(tab),
                    || release.mounted(tree),
                    &host,
                    &active,
                    &case,
                );
            }
        }
    }
}

/// Resolves its one captured archive mount once, then holds no reference to it.
struct OneMount(std::sync::Mutex<Option<OpenedLocation>>);
impl FilesystemProvider for OneMount {
    fn resolve(
        &self,
        _: &StoredLocation,
        _: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        self.0
            .lock()
            .unwrap()
            .take()
            .ok_or(ProviderError::StaleReference)
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        drop(Arc::into_inner(self));
        Ok(())
    }
}

#[test]
fn a_leased_archive_dropped_last_inside_a_quarantined_copy_hands_its_target_context_down() {
    let active = Arc::new(tokio::sync::Semaphore::new(2));
    let (host, probe) = crate::archives::Retirement::tainted_host_for_test(&active);
    let (tab, _, _) = crate::archives::Retirement::copy_mount_for_test(host);
    let cancel = Cancellation::default();
    let location = ArchiveLocation {
        mount: Some(tab.clone()),
        address: ArchiveAddress {
            provider: "taint-fixture".into(),
            format: None,
            container: std::env::temp_dir().join("tainted.zip"),
            source_version: vec![1],
            components: Vec::new(),
        },
        directory: 1,
        trail: vec![(1, "tainted.zip".into())],
    };
    let opened = captured_archive(&location, &cancel).unwrap();
    drop(location);
    let registry = Arc::new(ProviderRegistry::default());
    let address = StoredLocation::new(
        ProviderId::new("taint-fixture").unwrap(),
        NonZeroU32::MIN,
        Arc::from(&b"root"[..]),
    );
    registry
        .register(
            address.provider().clone(),
            Arc::new(OneMount(std::sync::Mutex::new(Some(opened)))),
        )
        .unwrap();
    let mounted = registry.resolve(&address, &cancel).unwrap();
    let target = mounted.resolve_target(mounted.root(), &cancel).unwrap();
    let ProviderTarget::Archive(archive) = target.value().unwrap() else {
        panic!("archive target expected")
    };
    // A clone of the resolved location, as a viewer or Copy source keeps one.
    let leased = archive.location.clone();
    // The target and the tree's location release ordinarily before the taint.
    nested::Context::default()
        .with(|| target.release_on_worker())
        .unwrap();
    nested::Context::default()
        .with(|| mounted.release_on_worker())
        .unwrap();
    probe.taint();
    assert!(Release::Worker.archive(tab));
    let host = || (probe.releases(), probe.stopped(), probe.dropped());
    assert_eq!(host(), (0, false, false));
    // Its last owner dropped inside a quarantined Copy: the source release fails, and
    // the target context it hands down passes the tree's capture reference on.
    let copy = Release::Quarantined.context();
    copy.with(|| drop(leased));
    assert!(copy.is_quarantined());
    assert_eq!(
        host(),
        (1, true, false),
        "the tree's capture pinned the session"
    );
    assert_eq!(active.available_permits(), 2);
}

#[test]
fn tainted_member_content_hands_its_guard_reference_down() {
    let active = Arc::new(tokio::sync::Semaphore::new(2));
    for release in Release::ALL {
        for member_first in [true, false] {
            let (tab, host) = tainted_tab(&active);
            let member = archive::member_for_taint_test(tab.clone());
            let case = format!("{release:?}, member first: {member_first}");
            // A second holder of the member, e.g. its captured file, only gives up its
            // count.
            assert!(release.content(member.clone()), "{case}");
            if member_first {
                release_shared_tainted_session(
                    || release.content(member),
                    || release.archive(tab),
                    &host,
                    &active,
                    &case,
                );
            } else {
                release_shared_tainted_session(
                    || release.archive(tab),
                    || release.content(member),
                    &host,
                    &active,
                    &case,
                );
            }
        }
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
