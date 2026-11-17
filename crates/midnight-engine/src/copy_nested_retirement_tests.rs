use super::*;
use crate::copy_nested_retirement::CheckedRetirement;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

struct Bomb(Arc<AtomicUsize>);
impl Drop for Bomb {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("opaque panic payload was dropped");
    }
}
#[derive(Debug)]
struct Archive {
    entered: mpsc::Sender<thread::ThreadId>,
    release: Mutex<mpsc::Receiver<()>>,
    drops: Arc<AtomicUsize>,
    bombs: Arc<AtomicUsize>,
    panic: bool,
}
impl Drop for Archive {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        self.entered.send(thread::current().id()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        if self.panic {
            std::panic::panic_any(Bomb(self.bombs.clone()));
        }
    }
}
impl ArchiveMount for Archive {
    fn id(&self) -> u64 {
        91
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
        if let Some(mount) = Arc::into_inner(self) {
            drop(mount);
        }
        Ok(())
    }
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

// This concrete fixture owns the engine's real archive TransferSource, backed
// by a GuardedMount whose native Archive implements the consuming release.
struct ArchivePayload {
    _source: Box<dyn TransferSource>,
}
impl CheckedRetirement for ArchivePayload {
    fn retirement_is_checked(&self) -> bool {
        true
    }
}
struct Fixture {
    io: Arc<midnight_work_budget::Gate>,
    namespace: Arc<Semaphore>,
    capacity: Arc<Semaphore>,
    pending: crate::copy_finished::Predecessors,
    _registry: crate::copy_namespace::Registry,
}
impl Fixture {
    fn resources() -> (Self, Resources) {
        Self::with_pin(None)
    }
    fn with_pin(pin: Option<Arc<dyn TransferNamespacePin>>) -> (Self, Resources) {
        let io = Arc::new(midnight_work_budget::Gate::new(1));
        let namespace = Arc::new(Semaphore::new(1));
        let capacity = Arc::new(Semaphore::new(1));
        let mut finished = crate::copy_finished::Registry::default();
        let (owner, previous) = finished.register().unwrap();
        let (_, pending) = finished.register().unwrap();
        let mut resources = Resources::admission(
            crate::copy_fence::CopyFence::default().register(),
            owner,
            capacity.clone().try_acquire_owned().unwrap(),
            previous,
        );
        resources.mutation = Some(namespace.clone().try_acquire_owned().unwrap());
        let permit = Arc::new(
            io.acquire(midnight_work_budget::Admission::Shared, || false)
                .unwrap(),
        );
        resources.attach_io(io.clone(), permit.clone()).unwrap();
        let registry = crate::copy_namespace::Registry::new_with_io(io.clone());
        let (claim, native) = registry.new_job(1).unwrap();
        resources.install_namespace(claim, native).unwrap();
        resources
            .namespace()
            .unwrap()
            .plan_union(
                vec![(TransferNamespaceAccess::Write, pin)],
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
        (
            Self {
                io,
                namespace,
                capacity,
                pending,
                _registry: registry,
            },
            resources,
        )
    }
    fn held(&self) {
        assert_eq!(self.io.snapshot().active_shared, 1);
        assert_eq!(self.namespace.available_permits(), 0);
        assert_eq!(self.capacity.available_permits(), 0);
        assert!(!self.pending.is_finished());
    }
    fn released(&self) {
        assert_eq!(self.io.snapshot().active_shared, 0);
        assert_eq!(self.namespace.available_permits(), 1);
        assert_eq!(self.capacity.available_permits(), 1);
        assert!(self.pending.is_finished());
    }
}
struct MountFixture {
    mount: Arc<dyn ArchiveMount>,
    archive_domain: crate::retirement::RetirementDomain,
    archive_capacity: Arc<Semaphore>,
    archive_closed_capacity: usize,
    lifetime: crate::task_lifetime::TaskLifetime,
    entered: mpsc::Receiver<thread::ThreadId>,
    release: Release,
    drops: Arc<AtomicUsize>,
    bombs: Arc<AtomicUsize>,
}
fn mount_fixture(panic: bool) -> MountFixture {
    let (entered, receive) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let drops = Arc::new(AtomicUsize::new(0));
    let bombs = Arc::new(AtomicUsize::new(0));
    let lifetime = crate::task_lifetime::TaskLifetime::testing(());
    let (mount, archive_domain, archive_capacity) =
        crate::archives::Retirement::copy_mount_for_test(Arc::new(Archive {
            entered,
            release: Mutex::new(wait),
            drops: drops.clone(),
            bombs: bombs.clone(),
            panic,
        }));
    let archive_closed_capacity = archive_capacity.available_permits() + 1;
    MountFixture {
        mount,
        archive_domain,
        archive_capacity,
        archive_closed_capacity,
        lifetime,
        entered: receive,
        release: Release(Some(release)),
        drops,
        bombs,
    }
}

#[test]
fn normal_non_domain_finish_waits_for_real_archive_last_reference_native_drop() {
    archive_last_reference(false);
}
#[test]
fn archive_native_bomb_quarantines_real_authority_and_later_retirement_still_runs() {
    if seal_tests::child_case(concat!(
        module_path!(),
        "::archive_native_bomb_quarantines_real_authority_and_later_retirement_still_runs"
    )) {
        return;
    }
    archive_last_reference(true);
    let (fixture, resources) = Fixture::resources();
    let mut benign = Scope::new((), resources);
    assert_eq!(benign.drain(), Ok(()));
    fixture.released();
}
fn archive_last_reference(panic: bool) {
    let (fixture, resources) = Fixture::resources();
    let mut mount = mount_fixture(panic);
    let payload = ArchivePayload {
        _source: archive_source::retirement_source_for_test(mount.mount.clone()),
    };
    drop(mount.mount);
    let (done, completed) = mpsc::channel();
    let thread = thread::spawn(move || {
        let id = thread::current().id();
        let mut scope = Scope::new(payload, resources);
        let result = scope.drain();
        done.send((id, result)).unwrap();
    });
    let entered = mount.entered.recv_timeout(Duration::from_secs(5));
    if let Ok(native_thread) = &entered {
        assert_eq!(
            *native_thread,
            thread.thread().id(),
            "native Drop must run inside the calling Copy seal, not the archive worker"
        );
        fixture.held();
        assert_eq!(
            mount.archive_capacity.available_permits(),
            mount.archive_closed_capacity - 1
        );
        assert!(completed.try_recv().is_err());
    }
    mount.release.now();
    let completion = completed.recv_timeout(Duration::from_secs(5));
    thread.join().unwrap();
    entered.unwrap();
    let (_, result) = completion.unwrap();
    assert_eq!(mount.drops.load(Ordering::SeqCst), 1);
    assert_eq!(mount.bombs.load(Ordering::SeqCst), 0);
    if panic {
        assert_eq!(result, Err(Quarantined));
        fixture.held();
        assert_eq!(
            mount.archive_capacity.available_permits(),
            mount.archive_closed_capacity - 1
        );
    } else {
        assert_eq!(result, Ok(()));
        fixture.released();
        assert_eq!(
            mount.archive_capacity.available_permits(),
            mount.archive_closed_capacity
        );
    }
    drop(mount.archive_domain);
    drop(mount.lifetime);
}

#[test]
fn copy_releases_its_archive_reference_without_waiting_for_other_tab_mount() {
    let (fixture, resources) = Fixture::resources();
    let mut mount = mount_fixture(false);
    let payload = ArchivePayload {
        _source: archive_source::retirement_source_for_test(mount.mount.clone()),
    };
    thread::spawn(move || {
        let mut scope = Scope::new(payload, resources);
        scope.drain().unwrap();
    })
    .join()
    .unwrap();
    fixture.released();
    assert_eq!(mount.drops.load(Ordering::SeqCst), 0);
    assert!(mount.entered.try_recv().is_err());
    mount.release.now();
    drop(mount.mount);
    mount.entered.recv_timeout(Duration::from_secs(5)).unwrap();
}

#[test]
fn unknown_other_domain_owner_is_retained_without_false_finished_acknowledgement() {
    let (fixture, mut resources) = Fixture::resources();
    let foreign = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    struct Deferred(Arc<AtomicUsize>);
    impl Drop for Deferred {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    resources.retain_provider(Box::new(
        foreign.try_reserve().unwrap().own(Deferred(drops.clone())),
    ));
    let result = thread::spawn(move || Scope::new((), resources).drain())
        .join()
        .unwrap();
    assert_eq!(result, Err(Quarantined));
    fixture.held();
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert!(foreign.try_reserve().is_err());
}

#[test]
fn archive_source_does_not_retain_unrelated_actor_service_lifetime() {
    let (fixture, resources) = Fixture::resources();
    let mut mount = mount_fixture(false);
    let payload = ArchivePayload {
        _source: archive_source::retirement_source_for_test(mount.mount.clone()),
    };
    drop(mount.mount);
    drop(mount.lifetime);
    mount.release.now();
    let result = thread::spawn(move || Scope::new(payload, resources).drain())
        .join()
        .unwrap();
    assert_eq!(result, Ok(()));
    assert_eq!(mount.drops.load(Ordering::SeqCst), 1);
    fixture.released();
}

#[test]
fn checked_last_service_lifetime_destroys_actual_other_domain_owner_before_finished() {
    struct LifetimePayload {
        _lifetime: crate::task_lifetime::TaskLifetime,
    }
    impl CheckedRetirement for LifetimePayload {
        fn retirement_is_checked(&self) -> bool {
            true
        }
    }
    struct Service(Arc<AtomicUsize>);
    impl CheckedRetirement for Service {
        fn retirement_is_checked(&self) -> bool {
            true
        }
    }
    impl Drop for Service {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let (fixture, resources) = Fixture::resources();
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let payload = LifetimePayload {
        _lifetime: crate::task_lifetime::TaskLifetime::checked(
            domain.try_reserve().unwrap().own(Service(drops.clone())),
        ),
    };
    let result = thread::spawn(move || Scope::new(payload, resources).drain())
        .join()
        .unwrap();
    assert_eq!(result, Ok(()));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(domain.try_reserve().is_ok());
    fixture.released();
}

#[test]
fn concurrent_consuming_archive_releases_assign_native_close_once() {
    let mut mount = mount_fixture(false);
    let first = mount.mount.clone();
    let second = mount.mount;
    let start = Arc::new(std::sync::Barrier::new(3));
    let (done, receive) = mpsc::channel();
    let threads: Vec<_> = [first, second]
        .into_iter()
        .map(|mount| {
            let start = start.clone();
            let done = done.clone();
            thread::spawn(move || {
                start.wait();
                done.send(mount.release_on_worker()).unwrap();
            })
        })
        .collect();
    start.wait();
    let first_result = receive.recv_timeout(Duration::from_secs(5));
    let entered = mount.entered.recv_timeout(Duration::from_secs(5));
    mount.release.now();
    let second_result = receive.recv_timeout(Duration::from_secs(5));
    for thread in threads {
        thread.join().unwrap();
    }
    first_result.unwrap().unwrap();
    entered.unwrap();
    second_result.unwrap().unwrap();
    assert_eq!(mount.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn nested_guarded_archive_releases_inline_while_its_archive_domain_is_occupied() {
    struct Blocker {
        entered: mpsc::Sender<()>,
        wait: mpsc::Receiver<()>,
    }
    impl Drop for Blocker {
        fn drop(&mut self) {
            self.entered.send(()).unwrap();
            self.wait.recv().unwrap();
        }
    }
    let mut mount = mount_fixture(false);
    let (entered, observe) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let mut unblock = Release(Some(release));
    let _ack = mount
        .archive_domain
        .try_reserve()
        .unwrap()
        .own(Blocker { entered, wait })
        .retire();
    observe.recv_timeout(Duration::from_secs(5)).unwrap();
    let (outer, _outer_domain, _outer_capacity) =
        crate::archives::Retirement::copy_mount_for_test(mount.mount);
    let (fixture, resources) = Fixture::resources();
    let payload = ArchivePayload {
        _source: archive_source::retirement_source_for_test(outer),
    };
    mount.release.now();
    let (done, observe) = mpsc::channel();
    let worker = thread::spawn(move || {
        done.send(Scope::new(payload, resources).drain()).unwrap();
    });
    let result = observe.recv_timeout(Duration::from_secs(5));
    let actual_drops = mount.drops.load(Ordering::SeqCst);
    let actual_archive_capacity = mount.archive_capacity.available_permits();
    unblock.now();
    worker.join().unwrap();
    result.unwrap().unwrap();
    assert_eq!(
        actual_drops, 1,
        "outer wrapper processing is not nested native retirement"
    );
    assert_eq!(actual_archive_capacity, mount.archive_closed_capacity);
    fixture.released();
}

#[test]
fn archive_release_refusal_is_visible_and_never_publishes_finished() {
    #[derive(Debug)]
    struct Refusal(Arc<AtomicUsize>);
    impl ArchiveMount for Refusal {
        fn id(&self) -> u64 {
            92
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
            self.0.fetch_add(1, Ordering::SeqCst);
            std::mem::forget(self);
            Err(FsError::from_io(
                "archive close",
                Path::new("retained-session"),
                io::ErrorKind::PermissionDenied.into(),
            ))
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let (fixture, mut resources) = Fixture::resources();
    let status = Arc::new(Mutex::new(CopyProgress::queued(
        OperationId(1),
        "target".into(),
        1,
        TransferIntent::Copy,
    )));
    resources.attach_status(status.clone());
    let payload = ArchivePayload {
        _source: archive_source::retirement_source_for_test(Arc::new(Refusal(calls.clone()))),
    };
    assert_eq!(Scope::new(payload, resources).drain(), Err(Quarantined));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        status
            .lock()
            .unwrap()
            .errors
            .iter()
            .any(|error| error.path == Path::new("retained-session"))
    );
    fixture.held();
}

#[test]
fn ordinary_operation_error_does_not_turn_confirmed_retirement_into_quarantine() {
    let (fixture, resources) = Fixture::resources();
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let mut owned = domain.try_reserve().unwrap().own(Scope::new((), resources));
    let result: Result<Result<(), FsError>, _> = run(&mut owned, |_| {
        Err(FsError::from_io(
            "operation refused",
            Path::new("original"),
            io::ErrorKind::PermissionDenied.into(),
        ))
    });
    assert!(result.unwrap().is_err());
    assert_eq!(retire_confirmed(owned), Ok(()));
    fixture.released();
}

#[test]
fn namespace_geometry_pin_does_not_implicitly_certify_native_retirement() {
    struct Pin(Arc<AtomicUsize>);
    impl TransferNamespacePin for Pin {
        fn refresh(&self, _: &Cancellation) -> Result<TransferNamespaceProbe, FsError> {
            Ok(TransferNamespaceProbe::Unknown)
        }
    }
    impl Drop for Pin {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let (fixture, resources) = Fixture::with_pin(Some(Arc::new(Pin(drops.clone()))));
    assert_eq!(Scope::new((), resources).drain(), Err(Quarantined));
    fixture.held();
    assert_eq!(drops.load(Ordering::SeqCst), 0);
}
