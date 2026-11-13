//! Lifecycle tests replace only native launch/exit, retaining the real manager and budget.
use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc::{self, Receiver, SyncSender},
};

const WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
struct TestProvider {
    id: String,
    revision: u64,
    document: bool,
}

fn provider(id: &str) -> TestProvider {
    TestProvider {
        id: id.into(),
        revision: 1,
        document: false,
    }
}

fn policy(revision: u64) -> Policy {
    Policy {
        disabled: Arc::from([]),
        associations: Arc::default(),
        catalog_revision: revision,
    }
}

#[derive(Default)]
struct Counters {
    launches: AtomicUsize,
    live: AtomicUsize,
    validations: AtomicUsize,
    reject_identity: AtomicBool,
    panic_identity: AtomicBool,
    validation_barrier: Mutex<Option<(SyncSender<()>, Receiver<()>)>>,
}

#[derive(Clone)]
struct FakeBackend {
    attempts: SyncSender<Attempt>,
    counters: Arc<Counters>,
}

struct Attempt {
    provider: TestProvider,
    finish: SyncSender<Outcome>,
}

enum Outcome {
    Ready {
        reaping: SyncSender<()>,
        exit: Receiver<()>,
    },
    Failed,
}

struct Exit {
    reaping: Receiver<()>,
    finish: SyncSender<()>,
}

impl Attempt {
    fn ready(self, hold_exit: bool) -> Exit {
        let (reaping, started) = mpsc::sync_channel(1);
        let (finish, exit) = mpsc::sync_channel(1);
        if !hold_exit {
            finish.send(()).unwrap();
        }
        self.finish.send(Outcome::Ready { reaping, exit }).unwrap();
        Exit {
            reaping: started,
            finish,
        }
    }

    fn fail(self) {
        self.finish.send(Outcome::Failed).unwrap();
    }
}

struct FakePrepared {
    provider: TestProvider,
    reaping: SyncSender<()>,
    exit: Receiver<()>,
    counters: Arc<Counters>,
}

impl Drop for FakePrepared {
    fn drop(&mut self) {
        let _ = self.reaping.try_send(());
        // Simulate native cleanup that cannot release its lifetime reservation early.
        let _ = self.exit.recv();
        self.counters.live.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Backend for FakeBackend {
    type Provider = TestProvider;
    type Prepared = FakePrepared;

    fn eligible(provider: &TestProvider, policy: &Policy) -> bool {
        policy.allows(&provider.id, provider.document, &["img".into()])
    }

    fn same_provider(left: &TestProvider, right: &TestProvider) -> bool {
        left == right
    }

    fn prepared_matches(prepared: &FakePrepared, provider: &TestProvider) -> bool {
        prepared.provider == *provider
    }

    fn prepare(&self, provider: &TestProvider, _cancel: &AtomicBool) -> io::Result<FakePrepared> {
        self.counters.launches.fetch_add(1, Ordering::AcqRel);
        self.counters.live.fetch_add(1, Ordering::AcqRel);
        let (finish, completed) = mpsc::sync_channel(1);
        self.attempts
            .send(Attempt {
                provider: provider.clone(),
                finish,
            })
            .unwrap();
        // A blocked CreateProcess call may return a child after cancellation.
        match completed.recv().unwrap() {
            Outcome::Ready { reaping, exit } => Ok(FakePrepared {
                provider: provider.clone(),
                reaping,
                exit,
                counters: self.counters.clone(),
            }),
            Outcome::Failed => {
                self.counters.live.fetch_sub(1, Ordering::AcqRel);
                Err(io::Error::other("controlled preparation failure"))
            }
        }
    }

    fn validate(&self, _prepared: &FakePrepared, _provider: &TestProvider) -> io::Result<()> {
        self.counters.validations.fetch_add(1, Ordering::AcqRel);
        let barrier = self.counters.validation_barrier.lock().unwrap().take();
        if let Some((started, release)) = barrier {
            started.send(()).unwrap();
            release.recv().unwrap();
        }
        assert!(
            !self.counters.panic_identity.load(Ordering::Acquire),
            "controlled identity panic"
        );
        if self.counters.reject_identity.load(Ordering::Acquire) {
            Err(io::Error::other("controlled stale package"))
        } else {
            Ok(())
        }
    }
}

struct Harness {
    manager: Option<Manager<FakeBackend>>,
    attempts: Receiver<Attempt>,
    counters: Arc<Counters>,
    exited: Receiver<()>,
}

impl Harness {
    fn new() -> Self {
        let (attempts, incoming) = mpsc::sync_channel(4);
        let (done, exited) = mpsc::sync_channel(1);
        let counters = Arc::new(Counters::default());
        let manager = Manager::new(
            FakeBackend {
                attempts,
                counters: counters.clone(),
            },
            move |run| {
                std::thread::spawn(move || {
                    run();
                    done.send(()).unwrap();
                });
            },
        );
        Self {
            manager: Some(manager),
            attempts: incoming,
            counters,
            exited,
        }
    }

    fn manager(&self) -> &Manager<FakeBackend> {
        self.manager.as_ref().unwrap()
    }

    fn attempt(&self) -> Attempt {
        self.attempts.recv_timeout(WAIT).unwrap()
    }

    fn configure(&self, providers: &[TestProvider]) {
        self.manager()
            .configure(Some(policy(1)), Arc::from(providers));
    }

    fn wait(&self, predicate: impl Fn(&State<FakeBackend>) -> bool) {
        let shared = &self.manager().control.shared;
        let deadline = Instant::now() + WAIT;
        let mut state = shared.lock();
        while !predicate(&state) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "manager did not reach the barrier");
            state = shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }

    fn ready(&self) {
        self.wait(|state| matches!(state.spare, Spare::Ready { .. }));
    }

    fn cold(&self, provider: &TestProvider) -> Acquisition<FakePrepared> {
        let acquired = self
            .manager()
            .acquire(provider, &AtomicBool::new(false))
            .unwrap();
        assert!(acquired.prepared.is_none());
        acquired
    }

    fn shutdown(&mut self) {
        self.manager.take();
        self.exited.recv_timeout(WAIT).unwrap();
        assert_eq!(self.counters.live.load(Ordering::Acquire), 0);
    }
}

#[test]
fn unknown_or_ineligible_startup_policy_preserves_the_cold_path() {
    let mut harness = Harness::new();
    let image = provider("image");
    let catalog: Arc<[_]> = Arc::from([image.clone()]);
    harness.manager().configure(None, catalog.clone());
    drop(harness.cold(&image));
    let mut disabled = policy(1);
    disabled.disabled = Arc::from(["image".into()]);
    harness.manager().configure(Some(disabled), catalog.clone());
    let mut unassociated = policy(2);
    Arc::make_mut(&mut unassociated.associations)
        .overrides
        .insert("image".into(), Vec::new());
    harness
        .manager()
        .configure(Some(unassociated), catalog.clone());
    let mut document = image.clone();
    document.document = true;
    harness.configure(&[document]);
    drop(harness.cold(&image));
    assert_eq!(harness.counters.launches.load(Ordering::Acquire), 0);
    assert!(harness.attempts.try_recv().is_err());
    harness.shutdown();
}

#[test]
fn ready_hit_is_consumed_once_and_concurrent_opened_requests_create_one_refill() {
    let mut harness = Harness::new();
    let image = provider("image");
    harness.configure(std::slice::from_ref(&image));
    let first_exit = harness.attempt().ready(false);
    harness.ready();
    let acquired = harness
        .manager()
        .acquire(&image, &AtomicBool::new(false))
        .unwrap();
    assert!(acquired.prepared.is_some());
    assert_eq!(harness.counters.launches.load(Ordering::Acquire), 1);
    assert_eq!(harness.counters.validations.load(Ordering::Acquire), 1);
    let manager = harness.manager();
    std::thread::scope(|scope| {
        for _ in 0..16 {
            scope.spawn(|| manager.opened(&image));
        }
    });
    let second = harness.attempt();
    assert_eq!(second.provider, image);
    let second_exit = second.ready(false);
    harness.ready();
    assert_eq!(harness.counters.launches.load(Ordering::Acquire), 2);
    assert!(harness.attempts.try_recv().is_err());
    drop(acquired);
    first_exit.reaping.recv_timeout(WAIT).unwrap();
    harness.shutdown();
    second_exit.reaping.recv_timeout(WAIT).unwrap();
}

#[test]
fn matching_inflight_startup_serves_demand_without_another_launch_even_with_free_capacity() {
    for existing in [0, 3] {
        let mut harness = Harness::new();
        let image = provider("image");
        let active: Vec<_> = (0..existing).map(|_| harness.cold(&image)).collect();
        harness.configure(std::slice::from_ref(&image));
        let attempt = harness.attempt();
        let manager = harness.manager().clone();
        let requested = image.clone();
        let (result, received) = mpsc::sync_channel(1);
        let demand = std::thread::spawn(move || {
            result
                .send(manager.acquire(&requested, &AtomicBool::new(false)))
                .unwrap();
        });
        harness.wait(|state| {
            state.demands == 1
                && state.children == existing + 1
                && matches!(&state.spare, Spare::Starting { cancel, .. } if !cancel.load(Ordering::Acquire))
        });
        assert!(received.try_recv().is_err());
        let exit = attempt.ready(false);
        let acquired = received.recv_timeout(WAIT).unwrap().unwrap();
        assert!(acquired.prepared.is_some());
        assert_eq!(harness.counters.launches.load(Ordering::Acquire), 1);
        demand.join().unwrap();
        drop(acquired);
        exit.reaping.recv_timeout(WAIT).unwrap();
        drop(active);
        harness.shutdown();
    }
}

#[test]
fn canceling_one_matching_waiter_preserves_startup_for_the_surviving_viewer() {
    let mut harness = Harness::new();
    let image = provider("image");
    let active: Vec<_> = (0..3).map(|_| harness.cold(&image)).collect();
    harness.configure(std::slice::from_ref(&image));
    let attempt = harness.attempt();
    let cancel = Arc::new(AtomicBool::new(false));
    let stopped = cancel.clone();
    let manager = harness.manager().clone();
    let first_provider = image.clone();
    let (first, first_result) = mpsc::sync_channel(1);
    let canceled = std::thread::spawn(move || {
        first
            .send(manager.acquire(&first_provider, &stopped))
            .unwrap();
    });
    let manager = harness.manager().clone();
    let (second, second_result) = mpsc::sync_channel(1);
    let survivor = std::thread::spawn(move || {
        second
            .send(manager.acquire(&image, &AtomicBool::new(false)))
            .unwrap();
    });
    harness.wait(|state| state.demands == 2 && state.children == 4);
    cancel.store(true, Ordering::Release);
    let error = first_result.recv_timeout(WAIT).unwrap().err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    canceled.join().unwrap();
    harness.wait(|state| state.demands == 1);
    {
        let state = harness.manager().control.shared.lock();
        assert_eq!(state.children, 4);
        assert!(
            matches!(&state.spare, Spare::Starting { cancel, .. } if !cancel.load(Ordering::Acquire))
        );
    }
    assert!(second_result.try_recv().is_err());
    let exit = attempt.ready(true);
    let acquired = second_result.recv_timeout(WAIT).unwrap().unwrap();
    assert!(acquired.prepared.is_some());
    survivor.join().unwrap();
    assert_eq!(harness.counters.launches.load(Ordering::Acquire), 1);
    assert!(harness.attempts.try_recv().is_err());
    let retire = std::thread::spawn(move || drop(acquired));
    exit.reaping.recv_timeout(WAIT).unwrap();
    harness.wait(|state| state.children == 4 && state.demands == 0);
    exit.finish.send(()).unwrap();
    retire.join().unwrap();
    harness.wait(|state| state.children == 3);
    drop(active);
    harness.shutdown();
}

#[test]
fn failed_matching_inflight_startup_falls_back_before_source_open() {
    let mut harness = Harness::new();
    let image = provider("image");
    harness.configure(std::slice::from_ref(&image));
    let attempt = harness.attempt();
    let manager = harness.manager().clone();
    let (result, received) = mpsc::sync_channel(1);
    let demand = std::thread::spawn(move || {
        result
            .send(manager.acquire(&image, &AtomicBool::new(false)))
            .unwrap();
    });
    harness.wait(|state| state.demands == 1);
    attempt.fail();
    let acquired = received.recv_timeout(WAIT).unwrap().unwrap();
    assert!(acquired.prepared.is_none());
    assert_eq!(harness.counters.launches.load(Ordering::Acquire), 1);
    demand.join().unwrap();
    harness.wait(|state| state.children == 1 && matches!(state.spare, Spare::Empty));
    drop(acquired);
    harness.shutdown();
}

#[test]
fn fourth_demand_waits_for_a_wrong_ready_child_to_actually_exit() {
    let mut harness = Harness::new();
    let image = provider("image");
    let other = provider("other");
    let active: Vec<_> = (0..3).map(|_| harness.cold(&other)).collect();
    harness.configure(&[image, other.clone()]);
    let exit = harness.attempt().ready(true);
    harness.ready();
    let manager = harness.manager().clone();
    let (result, received) = mpsc::sync_channel(1);
    let demand = std::thread::spawn(move || {
        result
            .send(manager.acquire(&other, &AtomicBool::new(false)))
            .unwrap();
    });
    exit.reaping.recv_timeout(WAIT).unwrap();
    harness.wait(|state| state.demands == 1 && state.children == 4);
    assert!(received.try_recv().is_err());
    exit.finish.send(()).unwrap();
    let acquired = received.recv_timeout(WAIT).unwrap().unwrap();
    assert!(acquired.prepared.is_none());
    demand.join().unwrap();
    drop(acquired);
    drop(active);
    harness.shutdown();
}

#[test]
fn canceled_native_startup_keeps_its_slot_through_return_and_reaping() {
    let mut harness = Harness::new();
    let image = provider("image");
    let other = provider("other");
    let active: Vec<_> = (0..3).map(|_| harness.cold(&other)).collect();
    harness.configure(&[image, other.clone()]);
    let attempt = harness.attempt();
    let manager = harness.manager().clone();
    let (result, received) = mpsc::sync_channel(1);
    let demand = std::thread::spawn(move || {
        result
            .send(manager.acquire(&other, &AtomicBool::new(false)))
            .unwrap();
    });
    harness.wait(|state| {
        state.demands == 1
            && state.children == 4
            && matches!(&state.spare, Spare::Starting { cancel, .. } if cancel.load(Ordering::Acquire))
    });
    assert!(received.try_recv().is_err());
    let exit = attempt.ready(true);
    exit.reaping.recv_timeout(WAIT).unwrap();
    harness.wait(|state| state.children == 4);
    assert!(received.try_recv().is_err());
    assert!(harness.attempts.try_recv().is_err());
    exit.finish.send(()).unwrap();
    let acquired = received.recv_timeout(WAIT).unwrap().unwrap();
    assert!(acquired.prepared.is_none());
    demand.join().unwrap();
    drop(acquired);
    drop(active);
    harness.shutdown();
}

#[test]
fn queued_foreground_demand_wins_released_capacity_before_refill() {
    let mut harness = Harness::new();
    let image = provider("image");
    let mut active: Vec<_> = (0..4).map(|_| harness.cold(&image)).collect();
    harness.configure(std::slice::from_ref(&image));
    let manager = harness.manager().clone();
    let requested = image.clone();
    let (result, received) = mpsc::sync_channel(1);
    let demand = std::thread::spawn(move || {
        result
            .send(manager.acquire(&requested, &AtomicBool::new(false)))
            .unwrap();
    });
    harness.wait(|state| state.demands == 1);
    drop(active.pop());
    let acquired = received.recv_timeout(WAIT).unwrap().unwrap();
    assert!(acquired.prepared.is_none());
    demand.join().unwrap();
    assert_eq!(harness.counters.launches.load(Ordering::Acquire), 0);
    drop(active.pop());
    harness.attempt().fail();
    harness.wait(|state| state.children == 3 && matches!(state.spare, Spare::Empty));
    drop(acquired);
    drop(active);
    harness.shutdown();
}

#[test]
fn canceling_waiting_demand_does_not_release_canceled_startup_capacity() {
    let mut harness = Harness::new();
    let image = provider("image");
    let active: Vec<_> = (0..3).map(|_| harness.cold(&image)).collect();
    harness.configure(std::slice::from_ref(&image));
    let attempt = harness.attempt();
    let manager = harness.manager().clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let stopped = cancel.clone();
    let (result, received) = mpsc::sync_channel(1);
    let demand = std::thread::spawn(move || {
        result.send(manager.acquire(&image, &stopped)).unwrap();
    });
    harness.wait(|state| state.demands == 1);
    // Global policy invalidation cancels the source-free launch independently of this caller.
    harness.manager().configure(None, Arc::from([]));
    cancel.store(true, Ordering::Release);
    let error = received.recv_timeout(WAIT).unwrap().err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    demand.join().unwrap();
    harness.wait(|state| state.demands == 0 && state.children == 4);
    let exit = attempt.ready(true);
    exit.reaping.recv_timeout(WAIT).unwrap();
    harness.wait(|state| state.children == 4);
    exit.finish.send(()).unwrap();
    harness.wait(|state| state.children == 3 && matches!(state.spare, Spare::Empty));
    drop(active);
    harness.shutdown();
}

#[test]
fn catalog_epoch_invalidates_ready_even_when_the_catalog_allocation_is_unchanged() {
    let mut harness = Harness::new();
    let image = provider("image");
    let catalog: Arc<[_]> = Arc::from([image.clone()]);
    harness
        .manager()
        .configure(Some(policy(1)), catalog.clone());
    let exit = harness.attempt().ready(true);
    harness.ready();
    harness
        .manager()
        .configure(Some(policy(2)), catalog.clone());
    exit.reaping.recv_timeout(WAIT).unwrap();
    assert!(harness.attempts.try_recv().is_err());
    exit.finish.send(()).unwrap();
    let replacement_exit = harness.attempt().ready(false);
    harness.ready();
    let acquired = harness
        .manager()
        .acquire(&image, &AtomicBool::new(false))
        .unwrap();
    assert!(acquired.prepared.is_some());
    assert_eq!(harness.counters.launches.load(Ordering::Acquire), 2);
    drop(acquired);
    replacement_exit.reaping.recv_timeout(WAIT).unwrap();
    harness.shutdown();
}

#[test]
fn disabled_policy_cancels_inflight_startup_without_adopting_its_late_child() {
    let mut harness = Harness::new();
    let image = provider("image");
    let catalog: Arc<[_]> = Arc::from([image.clone()]);
    harness
        .manager()
        .configure(Some(policy(1)), catalog.clone());
    let attempt = harness.attempt();
    let mut disabled = policy(1);
    disabled.disabled = Arc::from([image.id.clone()]);
    harness.manager().configure(Some(disabled), catalog);
    let exit = attempt.ready(false);
    exit.reaping.recv_timeout(WAIT).unwrap();
    harness.wait(|state| state.children == 0 && matches!(state.spare, Spare::Empty));
    drop(harness.cold(&image));
    assert_eq!(harness.counters.validations.load(Ordering::Acquire), 0);
    assert_eq!(harness.counters.launches.load(Ordering::Acquire), 1);
    harness.shutdown();
}

#[test]
fn changed_provider_identity_is_not_adopted_under_the_same_display_id() {
    let mut harness = Harness::new();
    let image = provider("image");
    harness.configure(std::slice::from_ref(&image));
    let exit = harness.attempt().ready(false);
    harness.ready();
    let mut replacement = image;
    replacement.revision = 2;
    drop(harness.cold(&replacement));
    exit.reaping.recv_timeout(WAIT).unwrap();
    assert_eq!(harness.counters.validations.load(Ordering::Acquire), 0);
    harness.shutdown();
}

#[test]
fn failed_prepare_does_not_retry_until_a_later_successful_open_requests_refill() {
    let mut harness = Harness::new();
    let image = provider("image");
    let catalog: Arc<[_]> = Arc::from([image.clone()]);
    harness
        .manager()
        .configure(Some(policy(1)), catalog.clone());
    harness.attempt().fail();
    harness.wait(|state| state.children == 0 && matches!(state.spare, Spare::Empty));
    harness.manager().configure(Some(policy(1)), catalog);
    let acquired = harness.cold(&image);
    assert!(harness.attempts.try_recv().is_err());
    harness.manager().opened(&image);
    harness.attempt().fail();
    harness.wait(|state| state.children == 1 && matches!(state.spare, Spare::Empty));
    assert_eq!(harness.counters.launches.load(Ordering::Acquire), 2);
    drop(acquired);
    harness.shutdown();
}

#[test]
fn stale_prepared_identity_returns_the_cold_path_under_the_existing_reservation() {
    let mut harness = Harness::new();
    let image = provider("image");
    harness.configure(std::slice::from_ref(&image));
    let exit = harness.attempt().ready(false);
    harness.ready();
    harness
        .counters
        .reject_identity
        .store(true, Ordering::Release);
    let acquired = harness.cold(&image);
    exit.reaping.recv_timeout(WAIT).unwrap();
    harness.wait(|state| state.children == 1);
    assert_eq!(harness.counters.launches.load(Ordering::Acquire), 1);
    assert_eq!(harness.counters.validations.load(Ordering::Acquire), 1);
    drop(acquired);
    harness.shutdown();
}

#[test]
fn catalog_invalidation_during_identity_validation_prevents_adoption_without_blocking_configure() {
    let mut harness = Harness::new();
    let image = provider("image");
    let catalog: Arc<[_]> = Arc::from([image.clone()]);
    harness
        .manager()
        .configure(Some(policy(1)), catalog.clone());
    let exit = harness.attempt().ready(false);
    harness.ready();
    let (started, validating) = mpsc::sync_channel(1);
    let (release, proceed) = mpsc::sync_channel(1);
    *harness.counters.validation_barrier.lock().unwrap() = Some((started, proceed));
    let manager = harness.manager().clone();
    let (result, received) = mpsc::sync_channel(1);
    let demand = std::thread::spawn(move || {
        result
            .send(manager.acquire(&image, &AtomicBool::new(false)))
            .unwrap();
    });
    validating.recv_timeout(WAIT).unwrap();
    // This must complete while filesystem identity validation remains blocked on another backend.
    let manager = harness.manager().clone();
    let (configured, reconfigured) = mpsc::sync_channel(1);
    let configure = std::thread::spawn(move || {
        manager.configure(None, catalog);
        configured.send(()).unwrap();
    });
    reconfigured.recv_timeout(WAIT).unwrap();
    release.send(()).unwrap();
    configure.join().unwrap();
    let acquired = received.recv_timeout(WAIT).unwrap().unwrap();
    assert!(acquired.prepared.is_none());
    demand.join().unwrap();
    exit.reaping.recv_timeout(WAIT).unwrap();
    harness.wait(|state| state.children == 1);
    drop(acquired);
    harness.shutdown();
}

#[test]
fn idle_expiry_reaps_once_without_automatic_rewarming() {
    let mut harness = Harness::new();
    let image = provider("image");
    let catalog: Arc<[_]> = Arc::from([image.clone()]);
    harness
        .manager()
        .configure(Some(policy(1)), catalog.clone());
    let exit = harness.attempt().ready(false);
    harness.ready();
    {
        let shared = &harness.manager().control.shared;
        let mut state = shared.lock();
        let Spare::Ready { expires, .. } = &mut state.spare else {
            panic!("test readiness barrier lost its child");
        };
        // Move the real owner's deadline into the past instead of sleeping for the idle timeout.
        *expires = Instant::now();
        shared.changed.notify_all();
    }
    exit.reaping.recv_timeout(WAIT).unwrap();
    harness.wait(|state| state.children == 0 && matches!(state.spare, Spare::Empty));
    harness.manager().configure(Some(policy(1)), catalog);
    assert_eq!(harness.counters.launches.load(Ordering::Acquire), 1);
    assert!(harness.attempts.try_recv().is_err());
    harness.shutdown();
}

#[test]
fn claimed_child_keeps_the_budget_until_its_destructor_finishes() {
    let mut harness = Harness::new();
    let image = provider("image");
    harness.configure(std::slice::from_ref(&image));
    let exit = harness.attempt().ready(true);
    harness.ready();
    let acquired = harness
        .manager()
        .acquire(&image, &AtomicBool::new(false))
        .unwrap();
    let active: Vec<_> = (0..3).map(|_| harness.cold(&image)).collect();
    let retire = std::thread::spawn(move || drop(acquired));
    exit.reaping.recv_timeout(WAIT).unwrap();
    harness.wait(|state| state.children == 4);
    exit.finish.send(()).unwrap();
    retire.join().unwrap();
    harness.wait(|state| state.children == 3);
    drop(active);
    harness.shutdown();
}

#[test]
fn panicking_identity_validation_keeps_capacity_reserved_until_native_drop_finishes() {
    let mut harness = Harness::new();
    let image = provider("image");
    harness.configure(std::slice::from_ref(&image));
    let exit = harness.attempt().ready(true);
    harness.ready();
    harness
        .counters
        .panic_identity
        .store(true, Ordering::Release);
    let manager = harness.manager().clone();
    let requested = image.clone();
    let (result, received) = mpsc::sync_channel(1);
    let demand = std::thread::spawn(move || {
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            manager.acquire(&requested, &AtomicBool::new(false))
        }))
        .is_err();
        result.send(panicked).unwrap();
    });
    exit.reaping.recv_timeout(WAIT).unwrap();
    harness.wait(|state| state.demands == 0);
    assert_eq!(harness.manager().control.shared.lock().children, 1);
    assert!(received.try_recv().is_err());
    let active: Vec<_> = (0..3).map(|_| harness.cold(&image)).collect();
    harness.wait(|state| state.children == 4);
    exit.finish.send(()).unwrap();
    assert!(received.recv_timeout(WAIT).unwrap());
    demand.join().unwrap();
    harness.wait(|state| state.children == 3);
    drop(active);
    harness.shutdown();
}

#[test]
fn final_handle_drop_signals_shutdown_before_blocked_native_startup_or_reap_returns() {
    let mut harness = Harness::new();
    harness.configure(&[provider("image")]);
    let attempt = harness.attempt();
    let manager = harness.manager.take().unwrap();
    let (dropped, received) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        drop(manager);
        dropped.send(()).unwrap();
    });
    received.recv_timeout(WAIT).unwrap();
    assert!(harness.exited.try_recv().is_err());
    let exit = attempt.ready(true);
    exit.reaping.recv_timeout(WAIT).unwrap();
    assert!(harness.exited.try_recv().is_err());
    exit.finish.send(()).unwrap();
    harness.exited.recv_timeout(WAIT).unwrap();
    assert_eq!(harness.counters.live.load(Ordering::Acquire), 0);
}
