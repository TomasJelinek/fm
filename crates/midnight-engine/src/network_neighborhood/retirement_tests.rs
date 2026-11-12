use super::*;
#[path = "retirement_fixture.rs"]
mod fixture;
use crate::retirement::RetirementDomain;
use fixture::*;

fn start(
    load: impl FnOnce() -> Result<Arc<dyn NetworkNeighborhood>, NetworkFailure> + Send + 'static,
) -> (
    Controller,
    JoinSet<()>,
    mpsc::Receiver<Event>,
    RetirementDomain,
) {
    let (sender, events) = mpsc::channel(8);
    let mut tasks = JoinSet::new();
    let (retirement, domain) = reservation();
    let controller = Controller::with_discovery(
        retirement,
        TaskLifetime::testing(()),
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        crate::test_snapshots(Default::default()),
        &mut tasks,
        load,
    );
    (controller, tasks, events, domain)
}
fn reservation() -> (Retirement, RetirementDomain) {
    let domain = RetirementDomain::try_start(RETIREMENT_CAPACITY, 0).unwrap();
    (
        Retirement {
            provider: domain.try_reserve().unwrap(),
        },
        domain,
    )
}

#[test]
fn network_retirement_covers_completed_unconsumed_native_discovery() {
    let runtime = runtime();
    let entered = runtime.enter();
    let (load, observer) = factory(Fake::default());
    let (retirement, domain) = reservation();
    let work = spawn_discovery(retirement, load);
    wait_finished(&work);
    assert!(
        observer.dropped.try_recv().is_err(),
        "unconsumed result still owns provider"
    );
    drop(entered);
    drop(runtime);
    drop(work);
    assert_worker_drop(&observer);
    assert!(
        domain.try_reserve().is_err(),
        "native destruction must retain the startup slot"
    );
}

#[test]
fn network_retirement_covers_final_controller_once_lock_release() {
    let runtime = runtime();
    let entered = runtime.enter();
    let (load, observer) = factory(Fake::default());
    let (controller, mut tasks, mut events, domain) = start(load);
    runtime.block_on(drain(&mut tasks));
    assert!(matches!(
        events.try_recv().unwrap(),
        Event::NetworkAvailability(true)
    ));
    assert!(controller.provider.get().unwrap().is_ok());
    drop(tasks);
    drop(entered);
    drop(runtime);
    assert!(observer.dropped.try_recv().is_err());
    drop(controller);
    assert_worker_drop(&observer);
    assert!(
        domain.try_reserve().is_err(),
        "native destruction must retain the startup slot"
    );
}

#[test]
fn network_retirement_covers_unpolled_navigation_after_controller_drop() {
    let runtime = runtime();
    let entered = runtime.enter();
    let (load, observer) = factory(Fake::default());
    let (mut controller, mut tasks, _events, domain) = start(load);
    runtime.block_on(drain(&mut tasks));
    controller.navigate(
        PaneId::Left,
        NavigationId(1),
        NetworkAction::OpenRoot,
        SortSpec::default(),
        &mut tasks,
    );
    drop(controller);
    assert!(
        observer.dropped.try_recv().is_err(),
        "unpolled task owns the discovered provider"
    );
    drop(tasks);
    drop(entered);
    drop(runtime);
    assert_worker_drop(&observer);
    assert!(
        domain.try_reserve().is_err(),
        "native destruction must retain the startup slot"
    );
}

#[test]
fn network_retirement_retains_admission_during_failed_or_panicking_creation_cleanup() {
    for panics in [false, true] {
        let runtime = runtime();
        let entered = runtime.enter();
        let (load, observer) = factory(Fake::default());
        let (retirement, domain) = reservation();
        let work = spawn_discovery(retirement, move || {
            let _opened = load()?;
            assert!(!panics, "network lifetime fixture creation unwind");
            Err(failure("network lifetime fixture creation failed"))
        });
        assert_worker_drop(&observer);
        assert!(
            domain.try_reserve().is_err(),
            "cleanup still owns native creation admission"
        );
        release(&observer._release.0);
        let result = runtime.block_on(work);
        if panics {
            assert!(result.is_err());
        } else {
            assert!(matches!(result, Ok(Err(_))));
        }
        assert!(
            domain.try_reserve().is_ok(),
            "failed creation must return unused admission"
        );
        drop(entered);
        drop(runtime);
    }
}

#[test]
fn network_retirement_failed_discovery_publishes_plain_failure_and_releases_slot() {
    let runtime = runtime();
    let entered = runtime.enter();
    let (controller, mut tasks, mut events, domain) = start(|| {
        Err(NetworkFailure::new(
            NetworkFailureKind::Unsupported,
            "fixture unavailable",
        ))
    });
    runtime.block_on(drain(&mut tasks));
    assert!(matches!(
        events.try_recv().unwrap(),
        Event::NetworkAvailability(false)
    ));
    assert_eq!(
        controller
            .provider
            .get()
            .unwrap()
            .as_ref()
            .err()
            .unwrap()
            .kind,
        NetworkFailureKind::Unsupported
    );
    assert!(domain.try_reserve().is_ok());
    drop(controller);
    drop(tasks);
    drop(entered);
    drop(runtime);
}

#[test]
fn network_retirement_native_request_keeps_provider_and_call_permit_after_shutdown() {
    let runtime = runtime();
    let entered = runtime.enter();
    let native_gate = gate();
    let _release_native = Release(native_gate.clone());
    let (started, ready) = oneshot::channel();
    let (load, observer) = factory(Fake {
        entered: Mutex::new(Some(started)),
        release: Some(native_gate.clone()),
        ..Default::default()
    });
    let (mut controller, mut tasks, _events, domain) = start(load);
    runtime.block_on(drain(&mut tasks));
    let budget = controller.budget.clone();
    controller.navigate(
        PaneId::Left,
        NavigationId(1),
        NetworkAction::OpenRoot,
        SortSpec::default(),
        &mut tasks,
    );
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), ready)
            .await
            .unwrap()
            .unwrap()
    });
    assert_eq!(budget.available_permits(), 1);
    drop(controller);
    drop(tasks);
    drop(entered);
    runtime.shutdown_timeout(Duration::from_millis(10));
    assert!(
        observer.dropped.try_recv().is_err(),
        "running call still borrows native provider"
    );
    assert_eq!(
        budget.available_permits(),
        1,
        "runtime shutdown must not release a running call"
    );
    assert!(domain.try_reserve().is_err());
    release(&native_gate);
    assert_worker_drop(&observer);
    assert!(
        domain.try_reserve().is_err(),
        "final native destroy still occupies its one slot"
    );
    release(&observer._release.0);
    wait_for_capacity(&domain, Some(&budget));
}

#[test]
fn network_retirement_cancelled_queued_creation_never_opens_provider() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc as sync_mpsc,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let entered = runtime.enter();
    let native_gate = gate();
    let _release_native = Release(native_gate.clone());
    let (started, ready) = sync_mpsc::channel();
    let held = native_gate.clone();
    let blocker = runtime.spawn_blocking(move || {
        started.send(()).unwrap();
        let _released = held
            .1
            .wait_while(held.0.lock().unwrap(), |released| !*released)
            .unwrap();
    });
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let created = Arc::new(AtomicUsize::new(0));
    let count = created.clone();
    let (retirement, domain) = reservation();
    let work = spawn_discovery(retirement, move || {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Fake::default()))
    });
    work.abort();
    drop(entered);
    runtime.shutdown_timeout(Duration::from_millis(10));
    drop(work);
    release(&native_gate);
    wait_finished(&blocker);
    drop(blocker);
    wait_for_capacity(&domain, None);
    assert_eq!(
        created.load(Ordering::SeqCst),
        0,
        "aborted queued factory must never run"
    );
}

fn wait_for_capacity(domain: &RetirementDomain, budget: Option<&Semaphore>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if budget.is_none_or(|budget| budget.available_permits() == 2)
            && domain.try_reserve().is_ok()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "retired native ownership did not release admission"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
