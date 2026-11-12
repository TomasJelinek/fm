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
fn watch_lifetime_scope_after_native_unwind_retires_off_caller() {
    let runtime = runtime();
    let entered = runtime.enter();
    let (subscription, subscription_drop) = sync_mpsc::channel();
    let (controller, mut tasks, _events, domain) = start(
        Arc::new(LifetimeFiles {
            panic_on_read: true,
            ..Default::default()
        }),
        Some(Arc::new(AffineNative(subscription))),
    );
    let handle = controller.handle();
    let right = controller.panes[1].clone();
    let (scope, dropped, _release) = retained_scope();
    handle.set_tree_scope(scope);
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), tasks.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        controller.shutdown();
        stop(&controller, &mut tasks).await;
    });
    let (created, destroyed) = subscription_drop
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        created, destroyed,
        "native subscription must unwind on its creating worker"
    );
    assert_ne!(destroyed, thread::current().id());
    assert!(
        handle.panes[0].state.lock().unwrap().tree.is_some(),
        "panic must bypass final drain"
    );
    drop(controller);
    drop(tasks);
    drop(entered);
    drop(runtime);
    assert!(dropped.try_recv().is_err());
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
