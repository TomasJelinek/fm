use super::*;
#[path = "lifetime_fixture.rs"]
mod fixture;
use crate::retirement::RetirementDomain;
use fixture::*;
use std::sync::mpsc as sync_mpsc;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn start(
    files: Arc<dyn FileSystem>,
    native: Option<Arc<dyn WatchService>>,
) -> (
    Controller,
    JoinSet<()>,
    mpsc::Receiver<Event>,
    RetirementDomain,
) {
    let (sender, events) = mpsc::channel(8);
    let mut tasks = JoinSet::new();
    let domain = RetirementDomain::try_start(RETIREMENT_CAPACITY, 0).unwrap();
    let retirement = Retirement {
        panes: std::array::from_fn(|_| domain.try_reserve().unwrap()),
    };
    let scope = TaskLifetime::testing((files.clone(), native.clone()));
    let controller = Controller::with_timing(
        retirement,
        scope,
        files,
        native,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
        timing(Duration::from_secs(60)),
    );
    (controller, tasks, events, domain)
}
fn retained_scope() -> (tree::TreeScope, sync_mpsc::Receiver<ThreadId>, Release) {
    let (entered, dropped) = sync_mpsc::channel();
    let gate = Gate::new();
    let files = Arc::new(LifetimeFiles {
        _probe: Some(Probe {
            caller: thread::current().id(),
            entered,
            gate: gate.clone(),
        }),
        ..Default::default()
    });
    (captured_scope(files), dropped, Release(gate))
}

#[test]
fn watch_lifetime_scope_before_first_poll_retires_off_caller() {
    let runtime = runtime();
    let entered = runtime.enter();
    let (controller, tasks, _events, domain) = start(Arc::new(LifetimeFiles::default()), None);
    let handle = controller.handle();
    let right = controller.panes[1].clone();
    let (scope, dropped, _release) = retained_scope();
    handle.set_tree_scope(scope);
    drop(controller);
    drop(tasks);
    drop(entered);
    drop(runtime);
    assert!(
        dropped.try_recv().is_err(),
        "retained handle must keep its mounted scope"
    );
    drop(handle);
    assert_ne!(
        dropped.recv_timeout(Duration::from_secs(5)).unwrap(),
        thread::current().id()
    );
    assert!(
        domain.try_reserve().is_err(),
        "the gated mount must retain its pane slot"
    );
    drop(right);
}

#[test]
fn watch_lifetime_unwinds_keep_subscriptions_and_scope_off_caller() {
    let runtime = runtime();
    let entered = runtime.enter();
    let (subscription, subscription_drop) = sync_mpsc::channel();
    // Every probe panics; the restart interval outlasts the test.
    let (controller, tasks, mut events, domain) = start(
        Arc::new(LifetimeFiles {
            panic_on_read: true,
            ..Default::default()
        }),
        Some(Arc::new(AffineNative(subscription))),
    );
    let handle = controller.handle();
    let right = controller.panes[1].clone();
    let (scope, dropped, release) = retained_scope();
    let stamp = scope.stamp;
    handle.set_tree_scope(scope);
    // The first restart is immediate; after the second panic the worker waits.
    let waiting = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("the worker must report its rate-limited restart")
            .expect("watch output")
    });
    assert!(
        matches!(
            waiting,
            Event::WatchStatus {
                view: WatchedView::Tree(view),
                status: WatchStatus::Restarting,
            } if view == stamp
        ),
        "{waiting:?}"
    );
    for _ in 0..2 {
        let (created, destroyed) = subscription_drop
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!(
            created, destroyed,
            "native subscription must unwind on its creating worker"
        );
        assert_ne!(destroyed, thread::current().id());
    }
    assert!(
        dropped.try_recv().is_err(),
        "the scope survives the unwinds"
    );
    // Stopped while it waits, the worker drains the scope on its own thread,
    // where the gated mount blocks.
    controller.shutdown();
    drop(controller);
    drop(tasks);
    drop(entered);
    drop(runtime);
    drop(handle);
    assert_ne!(
        dropped.recv_timeout(Duration::from_secs(5)).unwrap(),
        thread::current().id()
    );
    assert!(
        domain.try_reserve().is_err(),
        "the gated mount must retain its pane slot"
    );
    drop(release);
    drop(right);
}

#[test]
fn watch_lifetime_unpolled_task_retires_service_off_caller() {
    let runtime = runtime();
    let entered = runtime.enter();
    let (sender, dropped) = sync_mpsc::channel();
    let gate = Gate::new();
    let _release = Release(gate.clone());
    let files = Arc::new(LifetimeFiles {
        _probe: Some(Probe {
            caller: thread::current().id(),
            entered: sender,
            gate,
        }),
        ..Default::default()
    });
    let (controller, tasks, _events, _domain) = start(files, None);
    drop(controller);
    drop(tasks);
    drop(entered);
    drop(runtime);
    assert_ne!(
        dropped.recv_timeout(Duration::from_secs(5)).unwrap(),
        thread::current().id()
    );
}
