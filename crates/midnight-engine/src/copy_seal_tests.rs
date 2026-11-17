//! Copy unwind containment retains admission and execution authority.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Deliberately broken Bomb handling must fail one child, not abort its siblings.
pub(crate) fn child_case(qualified: &str) -> bool {
    let (_, name) = qualified.split_once("::").expect("qualified test name");
    const CHILD: &str = "MIDNIGHT_COPY_SEAL_TEST_CHILD";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        return false;
    }
    struct Child(Option<std::process::Child>);
    impl Drop for Child {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    let mut child = Child(Some(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(CHILD, name)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while child.0.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "Copy seal child did not finish: {name}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.0.take().unwrap().wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed;"),
        "Copy seal child failed or matched no test: {name}\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

struct Bomb(Arc<AtomicUsize>);
impl Drop for Bomb {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("a caught panic payload must never be destroyed");
    }
}
struct Payload {
    cleanups: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    bombs: Arc<AtomicUsize>,
    panic_cleanup: bool,
    panic_drop: bool,
}
impl CheckedRetirement for Payload {
    fn retirement_is_checked(&self) -> bool {
        true
    }
}
impl Payload {
    fn cleanup(&mut self) {
        self.cleanups.fetch_add(1, Ordering::SeqCst);
        if self.panic_cleanup {
            std::panic::panic_any(Bomb(self.bombs.clone()));
        }
    }
}
impl Drop for Payload {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        if self.panic_drop {
            std::panic::panic_any(Bomb(self.bombs.clone()));
        }
    }
}
struct Fixture {
    io: Arc<midnight_work_budget::Gate>,
    namespace: Arc<Semaphore>,
    capacity: Arc<Semaphore>,
    earlier: crate::copy_finished::Predecessors,
    cleanups: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    bombs: Arc<AtomicUsize>,
}
impl Fixture {
    fn new(panic_cleanup: bool, panic_drop: bool) -> (Self, Scope<Payload>) {
        let io = Arc::new(midnight_work_budget::Gate::new(1));
        let namespace = Arc::new(Semaphore::new(1));
        let capacity = Arc::new(Semaphore::new(1));
        let mut registry = crate::copy_finished::Registry::default();
        let (finished, predecessors) = registry.register().unwrap();
        let (_later, earlier) = registry.register().unwrap();
        let mut resources = Resources::admission(
            crate::copy_fence::CopyFence::default().register(),
            finished,
            capacity.clone().try_acquire_owned().unwrap(),
            predecessors,
        );
        resources.mutation = Some(namespace.clone().try_acquire_owned().unwrap());
        resources.real_io = Some(Arc::new(
            io.acquire(midnight_work_budget::Admission::Shared, || false)
                .unwrap(),
        ));
        let fixture = Self {
            io,
            namespace,
            capacity,
            earlier,
            cleanups: Arc::default(),
            drops: Arc::default(),
            bombs: Arc::default(),
        };
        let payload = Payload {
            cleanups: fixture.cleanups.clone(),
            drops: fixture.drops.clone(),
            bombs: fixture.bombs.clone(),
            panic_cleanup,
            panic_drop,
        };
        (
            fixture,
            Scope::with_cleanup(payload, resources, Payload::cleanup),
        )
    }
    fn held(&self) {
        assert_eq!(self.io.snapshot().active_shared, 1);
        assert_eq!(self.namespace.available_permits(), 0);
        assert_eq!(self.capacity.available_permits(), 0);
        assert!(!self.earlier.is_finished());
        assert_eq!(self.bombs.load(Ordering::SeqCst), 0);
    }
    fn released(&self) {
        assert_eq!(self.io.snapshot().active_shared, 0);
        assert_eq!(self.namespace.available_permits(), 1);
        assert_eq!(self.capacity.available_permits(), 1);
        assert!(self.earlier.is_finished());
    }
}

#[test]
fn borrowed_cleanup_bomb_retains_payload_and_all_real_guards() {
    if child_case(concat!(
        module_path!(),
        "::borrowed_cleanup_bomb_retains_payload_and_all_real_guards"
    )) {
        return;
    }
    let (fixture, mut scope) = Fixture::new(true, false);
    assert_eq!(scope.drain(), Err(Quarantined));
    drop(scope);
    fixture.held();
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 0);
}

#[test]
fn destructor_bomb_never_releases_guards_or_drops_the_panic_payload() {
    if child_case(concat!(
        module_path!(),
        "::destructor_bomb_never_releases_guards_or_drops_the_panic_payload"
    )) {
        return;
    }
    let (fixture, mut scope) = Fixture::new(false, true);
    assert_eq!(scope.drain(), Err(Quarantined));
    drop(scope);
    fixture.held();
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn native_step_bomb_is_contained_before_the_blocking_task_result() {
    if child_case(concat!(
        module_path!(),
        "::native_step_bomb_is_contained_before_the_blocking_task_result"
    )) {
        return;
    }
    let (fixture, scope) = Fixture::new(false, false);
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let mut owned = domain.try_reserve().unwrap().own(scope);
    let result: Result<(), _> = run(&mut owned, |payload| {
        std::panic::panic_any(Bomb(payload.bombs.clone()));
    });
    assert_eq!(result, Err(Quarantined));
    drop(owned);
    fixture.held();
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 0);
}

#[test]
fn consuming_map_bomb_keeps_authority_outside_the_panicking_closure() {
    if child_case(concat!(
        module_path!(),
        "::consuming_map_bomb_keeps_authority_outside_the_panicking_closure"
    )) {
        return;
    }
    let (fixture, scope) = Fixture::new(false, false);
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let owned = domain.try_reserve().unwrap().own(scope);
    let result: Result<Owned<()>, _> = map(owned, |payload| {
        std::panic::panic_any(Bomb(payload.bombs.clone()));
    });
    assert!(matches!(result, Err(Quarantined)));
    fixture.held();
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 1);
}

struct Retention {
    order: Arc<Mutex<Vec<&'static str>>>,
    name: &'static str,
    bombs: Option<Arc<AtomicUsize>>,
}
impl CheckedRetirement for Retention {
    fn retirement_is_checked(&self) -> bool {
        true
    }
}
impl Drop for Retention {
    fn drop(&mut self) {
        self.order.lock().unwrap().push(self.name);
        if let Some(bombs) = &self.bombs {
            std::panic::panic_any(Bomb(bombs.clone()));
        }
    }
}

#[test]
fn retained_native_bomb_stops_before_remaining_native_and_provider_owners() {
    if child_case(concat!(
        module_path!(),
        "::retained_native_bomb_stops_before_remaining_native_and_provider_owners"
    )) {
        return;
    }
    let (fixture, mut scope) = Fixture::new(false, false);
    let order = Arc::new(Mutex::new(Vec::new()));
    for (name, bombs) in [
        ("native-first", Some(fixture.bombs.clone())),
        ("native-last", None),
    ] {
        scope.resources_mut().retain_native(Box::new(Retention {
            order: order.clone(),
            name,
            bombs,
        }));
    }
    scope
        .resources_mut()
        .retain_checked_provider(Box::new(Retention {
            order: order.clone(),
            name: "provider",
            bombs: None,
        }));
    assert_eq!(scope.drain(), Err(Quarantined));
    drop(scope);
    assert_eq!(*order.lock().unwrap(), ["native-first"]);
    fixture.held();
}

#[test]
fn successful_drain_is_idempotent_and_closes_native_before_provider() {
    let (fixture, mut scope) = Fixture::new(false, false);
    let order = Arc::new(Mutex::new(Vec::new()));
    scope.resources_mut().retain_native(Box::new(Retention {
        order: order.clone(),
        name: "native",
        bombs: None,
    }));
    scope
        .resources_mut()
        .retain_checked_provider(Box::new(Retention {
            order: order.clone(),
            name: "provider",
            bombs: None,
        }));
    assert_eq!(scope.drain(), Ok(()));
    assert_eq!(scope.drain(), Ok(()));
    drop(scope);
    assert_eq!(*order.lock().unwrap(), ["native", "provider"]);
    assert_eq!(fixture.cleanups.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 1);
    fixture.released();
}

#[test]
fn retirement_worker_processes_benign_scope_after_quarantined_cleanup() {
    if child_case(concat!(
        module_path!(),
        "::retirement_worker_processes_benign_scope_after_quarantined_cleanup"
    )) {
        return;
    }
    let domain = crate::retirement::RetirementDomain::try_start(2, 0).unwrap();
    let (fixture, scope) = Fixture::new(true, false);
    let (benign, next) = Fixture::new(false, false);
    let first = domain.try_reserve().unwrap().own(scope);
    let second = domain.try_reserve().unwrap().own(next);
    let first_ack = first.retire();
    let second_ack = second.retire();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), first_ack)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), second_ack)
            .await
            .unwrap()
            .unwrap();
    });
    fixture.held();
    benign.released();
    assert_eq!(benign.cleanups.load(Ordering::SeqCst), 1);
    assert_eq!(benign.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn armed_subvalue_survives_a_borrowed_operation_unwind_without_destructor() {
    if child_case(concat!(
        module_path!(),
        "::armed_subvalue_survives_a_borrowed_operation_unwind_without_destructor"
    )) {
        return;
    }
    let (fixture, scope) = Fixture::new(false, false);
    let value = seal::Armed::new(scope);
    let result: Result<(), _> = seal::checked(|| {
        std::panic::panic_any(Bomb(value.bombs.clone()));
    });
    assert_eq!(result, Err(Quarantined));
    drop(value);
    fixture.held();
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 0);
}

#[test]
fn delivery_tail_retains_only_original_admission_capacity_after_native_close() {
    let (fixture, mut scope) = Fixture::new(false, false);
    let tail = scope.resources_mut().delivery_tail();
    assert_eq!(scope.drain(), Ok(()));
    assert!(fixture.earlier.is_finished());
    assert_eq!(fixture.io.snapshot().active_shared, 0);
    assert_eq!(fixture.namespace.available_permits(), 1);
    assert_eq!(fixture.capacity.available_permits(), 0);
    drop(scope);
    drop(tail);
    fixture.released();
}
