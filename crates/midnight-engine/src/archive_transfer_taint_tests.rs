use super::*;
use crate::copy_nested_retirement as nested;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Read,
    List,
    Validate,
}
#[derive(Debug)]
struct Mount {
    call: Call,
    taint_on_call: bool,
    fail: bool,
    cancel_on_call: bool,
    tainted: AtomicBool,
    calls: AtomicUsize,
    releases: AtomicUsize,
}
impl Mount {
    fn result(&self, call: Call, cancel: Option<&Cancellation>) -> Result<(), FsError> {
        if call != self.call {
            return Ok(());
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.taint_on_call {
            self.tainted.store(true, Ordering::Release);
        }
        if self.cancel_on_call
            && let Some(cancel) = cancel
        {
            cancel.cancel();
        }
        if self.fail {
            Err(error(
                Path::new("member"),
                io::ErrorKind::PermissionDenied,
                "ordinary archive failure",
            ))
        } else {
            Ok(())
        }
    }
}
impl ArchiveMount for Mount {
    fn cleanup_unconfirmed(&self) -> bool {
        self.tainted.load(Ordering::Acquire)
    }
    fn id(&self) -> u64 {
        9
    }
    fn validate(&self) -> Result<(), FsError> {
        self.result(Call::Validate, None)
    }
    fn read(&self, _: u64, _: u64, _: u32, cancel: &Cancellation) -> Result<Vec<u8>, FsError> {
        self.result(Call::Read, Some(cancel))?;
        Ok(Vec::new())
    }
    fn list(&self, _: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.result(Call::List, Some(cancel))?;
        Ok(Vec::new())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        self.releases.fetch_add(1, Ordering::SeqCst);
        if self.cleanup_unconfirmed() {
            std::mem::forget(self);
            return Err(nested::unconfirmed());
        }
        let _ = Arc::into_inner(self);
        Ok(())
    }
}
fn mount(call: Call, taint: bool, fail: bool, cancel: bool) -> Arc<Mount> {
    Arc::new(Mount {
        call,
        taint_on_call: taint,
        fail,
        cancel_on_call: cancel,
        tainted: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        releases: AtomicUsize::new(0),
    })
}
fn source(mount: Arc<dyn ArchiveMount>, directory: bool) -> Source {
    let node = ArchiveNode {
        id: VirtualEntryId {
            mount: mount.id(),
            entry: 2,
        },
        parent: 1,
        raw_name: Arc::from(&b"member"[..]),
        display_name: "member".into(),
        kind: if directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        size: Some(0),
        modified: None,
        readable: true,
    };
    Source::new(MountReference::new(mount), node, 0, traversal()).unwrap()
}

#[test]
fn archive_read_observes_taint_before_propagating_error_or_success() {
    for fail in [true, false] {
        let mount = mount(Call::Read, true, fail, false);
        let mut source = source(mount.clone(), false);
        let context = nested::Context::default();
        let result = context.with(|| source.read_chunk(&mut [0], &Cancellation::default()));
        assert_eq!(result.is_err(), fail);
        if let Err(error) = result {
            assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
        }
        assert!(context.is_quarantined());
        assert_eq!(mount.calls.load(Ordering::SeqCst), 1);
        assert_eq!(mount.releases.load(Ordering::SeqCst), 0);
        context.with(|| drop(source));
        assert!(context.is_quarantined());
    }
}

#[test]
fn archive_listing_and_validation_failures_record_taint_before_question_mark() {
    for call in [Call::List, Call::Validate] {
        let mount = mount(call, true, true, false);
        let mut source = source(mount.clone(), true);
        let context = nested::Context::default();
        let result = context.with(|| source.next_child(&Cancellation::default()));
        assert!(matches!(result, Err(ref error) if error.kind == io::ErrorKind::PermissionDenied));
        assert!(context.is_quarantined());
        assert_eq!(mount.calls.load(Ordering::SeqCst), 1);
        assert_eq!(mount.releases.load(Ordering::SeqCst), 0);
        context.with(|| drop(source));
    }
}

#[test]
fn ordinary_archive_errors_and_cancellation_with_false_status_remain_confirmed() {
    for call in [Call::Read, Call::List, Call::Validate] {
        let mount = mount(call, false, true, false);
        let mut source = source(mount, call != Call::Read);
        let context = nested::Context::default();
        context.with(|| {
            if call == Call::Read {
                assert!(
                    source
                        .read_chunk(&mut [0], &Cancellation::default())
                        .is_err()
                );
            } else {
                assert!(source.next_child(&Cancellation::default()).is_err());
            }
            assert!(!context.is_quarantined());
            drop(source);
        });
        assert!(!context.is_quarantined());
    }
    let mount = mount(Call::Read, false, false, false);
    let mut source = source(mount.clone(), false);
    let cancel = Cancellation::default();
    cancel.cancel();
    let context = nested::Context::default();
    let result = context.with(|| source.read_chunk(&mut [0], &cancel));
    assert!(matches!(result, Err(error) if error.kind == io::ErrorKind::Interrupted));
    assert!(!context.is_quarantined());
    assert_eq!(mount.calls.load(Ordering::SeqCst), 0);
    context.with(|| drop(source));
}

#[test]
fn later_read_and_cancel_cannot_hide_previously_observed_archive_taint() {
    for cancelled in [true, false] {
        let mount = mount(Call::Read, false, false, false);
        mount.tainted.store(true, Ordering::Release);
        let mut source = source(mount, false);
        let cancel = Cancellation::default();
        if cancelled {
            cancel.cancel();
        }
        let context = nested::Context::default();
        let result = context.with(|| source.read_chunk(&mut [0], &cancel));
        assert_eq!(result.is_err(), cancelled);
        assert!(context.is_quarantined());
        context.with(|| drop(source));
    }
}

#[test]
fn cancellation_set_by_archive_call_does_not_hide_its_taint() {
    let mount = mount(Call::Read, true, false, true);
    let mut source = source(mount, false);
    let cancel = Cancellation::default();
    let context = nested::Context::default();
    assert_eq!(
        context
            .with(|| source.read_chunk(&mut [0], &cancel))
            .unwrap(),
        0
    );
    assert!(cancel.is_cancelled());
    assert!(context.is_quarantined());
    context.with(|| drop(source));
}

#[test]
fn shared_archive_read_taint_keeps_real_copy_authority_and_finished_pending() {
    use super::super::ownership::{self, Resources, Scope};
    use midnight_work_budget::{Admission, Gate};
    use tokio::sync::Semaphore;
    struct Payload(Source);
    impl nested::CheckedRetirement for Payload {
        // This fixture creates the real archive Source from explicit consuming
        // wrappers; it owns no deferred or opaque provider outside that bridge.
        fn retirement_is_checked(&self) -> bool {
            true
        }
    }
    let leaf = mount(Call::Read, true, true, false);
    let (guarded, archive_domain, _) =
        crate::archives::Retirement::copy_mount_for_test(leaf.clone());
    let shared = crate::tree_actor::Session::archive_for_taint_test(
        crate::providers::MountedTarget::archive_for_taint_test(guarded),
    );
    let payload = Payload(source(shared.clone(), false));
    let io = Arc::new(Gate::new(1));
    let namespace = Arc::new(Semaphore::new(1));
    let capacity = Arc::new(Semaphore::new(1));
    let mut finished = crate::copy_finished::Registry::default();
    let (owner, predecessors) = finished.register().unwrap();
    let (_, pending) = finished.register().unwrap();
    let mut resources = Resources::admission(
        crate::copy_fence::CopyFence::default().register(),
        owner,
        capacity.clone().try_acquire_owned().unwrap(),
        predecessors,
    );
    resources.mutation = Some(namespace.clone().try_acquire_owned().unwrap());
    let permit = Arc::new(io.acquire(Admission::Shared, || false).unwrap());
    resources.attach_io(io.clone(), permit.clone()).unwrap();
    let registry = crate::copy_namespace::Registry::new_with_io(io.clone());
    let (claim, native) = registry.new_job(1).unwrap();
    resources.install_namespace(claim, native).unwrap();
    resources
        .namespace()
        .unwrap()
        .plan_union(
            vec![(midnight_core::TransferNamespaceAccess::Write, None)],
            permit,
            &Cancellation::default(),
        )
        .unwrap();
    assert!(
        resources
            .namespace()
            .unwrap()
            .try_grant(&Cancellation::default())
            .unwrap()
    );
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let mut owned = domain
        .try_reserve()
        .unwrap()
        .own(Scope::new(payload, resources));
    assert!(
        ownership::run(&mut owned, |payload| payload
            .0
            .read_chunk(&mut [0], &Cancellation::default()))
        .is_err()
    );
    assert!(ownership::retire_confirmed(owned).is_err());
    assert_eq!(leaf.calls.load(Ordering::SeqCst), 1);
    assert_eq!(leaf.releases.load(Ordering::SeqCst), 0);
    assert!(shared.cleanup_unconfirmed());
    assert_eq!(io.snapshot().active_shared, 1);
    assert_eq!(namespace.available_permits(), 0);
    assert_eq!(capacity.available_permits(), 0);
    assert!(!pending.is_finished());
    drop(shared);
    drop(archive_domain);
    drop(domain);
}
