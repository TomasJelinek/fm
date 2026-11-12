use super::*;
use std::{
    fs::File,
    sync::{Mutex, mpsc},
    thread::{self, ThreadId},
    time::Duration,
};

#[derive(Debug)]
struct DropReport(mpsc::Sender<ThreadId>);
impl Drop for DropReport {
    fn drop(&mut self) {
        let _ = self.0.send(thread::current().id());
    }
}
#[derive(Debug)]
struct NativeMount {
    // Observe after the actual native handle has closed.
    _file: File,
    _drop: DropReport,
    entered: Mutex<Option<mpsc::Sender<()>>>,
    cancel_on_list: bool,
    release: Mutex<Option<mpsc::Receiver<()>>>,
}
impl ArchiveMount for NativeMount {
    fn id(&self) -> u64 {
        71
    }
    fn list(&self, _: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        if self.cancel_on_list {
            cancel.cancel();
        }
        if let Some(entered) = self.entered.lock().unwrap().take() {
            let _ = entered.send(());
        }
        if let Some(release) = self.release.lock().unwrap().take() {
            release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Ok(Vec::new())
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("navigation must not read member content")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    controller: Controller,
    events: tokio::sync::mpsc::Receiver<Event>,
    tasks: JoinSet<()>,
    dropped: mpsc::Receiver<ThreadId>,
    entered: mpsc::Receiver<()>,
    action: Option<ArchiveAction>,
    release: Option<mpsc::Sender<()>>,
}
impl Fixture {
    fn new(cancel_on_list: bool) -> Self {
        Self::build(cancel_on_list, false)
    }
    fn build(cancel_on_list: bool, gated: bool) -> Self {
        let root = tempfile::Builder::new()
            .prefix("archive-retirement-")
            .tempdir()
            .unwrap();
        let path = root.path().join("owned.zip");
        let (report, dropped) = mpsc::channel();
        let (started, entered) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mount: Arc<dyn ArchiveMount> = Arc::new(NativeMount {
            _file: File::create(&path).unwrap(),
            _drop: DropReport(report),
            entered: Mutex::new(Some(started)),
            cancel_on_list,
            release: Mutex::new(gated.then_some(wait)),
        });
        let action = ArchiveAction::List(ArchiveLocation {
            mount: Some(mount),
            address: ArchiveAddress {
                format: Some(ArchiveFormat::Zip),
                provider: "test.retirement".into(),
                container: path,
                source_version: vec![1],
                components: Vec::new(),
            },
            directory: 1,
            trail: vec![(1, "owned.zip".into())],
        });
        let (sender, events) = tokio::sync::mpsc::channel(1);
        Self {
            _root: root,
            controller: Controller::testing(
                Outbox {
                    sender,
                    wakeup: Arc::new(|| {}),
                },
                crate::test_snapshots(Default::default()),
                Arc::new(Semaphore::new(2)),
            ),
            events,
            tasks: JoinSet::new(),
            dropped,
            entered,
            action: Some(action),
            release: gated.then_some(release),
        }
    }
    async fn navigate(&mut self) {
        self.controller
            .navigate(
                PaneId::Left,
                NavigationId(1),
                self.action.take().unwrap(),
                SortSpec::default(),
                &mut self.tasks,
            )
            .await;
    }
    fn assert_native_cleanup_off_caller(&self, boundary: &str) {
        let dropped = self.dropped.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(dropped, thread::current().id(), "{boundary}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn archive_retirement_unpolled_list_action_does_not_drop_native_mount_on_executor() {
    let mut fixture = Fixture::new(false);
    fixture.navigate().await;
    // Intake returned synchronously; no spawned task has been polled yet.
    fixture.tasks.abort_all();
    while fixture.tasks.join_next().await.is_some() {}
    assert!(fixture.entered.try_recv().is_err());
    fixture
        .assert_native_cleanup_off_caller("unpolled List action released native mount on executor");
}

#[tokio::test(flavor = "current_thread")]
async fn archive_retirement_busy_refusal_does_not_drop_input_mount_on_executor() {
    let mut fixture = Fixture::new(false);
    fixture.controller.budget = Arc::new(Semaphore::new(0));
    fixture.navigate().await;
    let event = tokio::time::timeout(Duration::from_secs(5), fixture.events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, Event::ArchiveListed { result: Err(_), .. }));
    while fixture.tasks.join_next().await.is_some() {}
    assert!(
        fixture.entered.try_recv().is_err(),
        "refused work reached provider"
    );
    fixture.assert_native_cleanup_off_caller("busy refusal released native mount on executor");
}

#[test]
fn archive_retirement_completed_unconsumed_listing_keeps_native_owner_guarded() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let mut fixture = Fixture::build(true, true);
    runtime.block_on(async {
        fixture.navigate().await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if fixture.entered.try_recv().is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    });
    // The sole worker's later marker proves that listing has returned without
    // giving its async waiter another poll. Cancellation prevented root adoption.
    fixture.release.take().unwrap().send(()).unwrap();
    let (finished, marker) = mpsc::channel();
    let barrier = runtime.spawn_blocking(move || finished.send(()).unwrap());
    marker.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        fixture.controller.snapshots.lock().unwrap()[0]
            .tree_root
            .is_none()
    );
    fixture.tasks.abort_all();
    runtime.block_on(async { while fixture.tasks.join_next().await.is_some() {} });
    drop(barrier);
    runtime.shutdown_timeout(Duration::ZERO);
    fixture
        .assert_native_cleanup_off_caller("completed unconsumed listing released mount on caller");
}

#[tokio::test(flavor = "current_thread")]
async fn archive_retirement_delivered_stale_listing_keeps_native_owner_guarded() {
    let mut fixture = Fixture::new(true);
    fixture.navigate().await;
    let event = tokio::time::timeout(Duration::from_secs(5), fixture.events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, Event::ArchiveListed { result: Ok(_), .. }));
    while fixture.tasks.join_next().await.is_some() {}
    assert!(
        fixture.controller.snapshots.lock().unwrap()[0]
            .tree_root
            .is_none()
    );
    drop(event);
    fixture.assert_native_cleanup_off_caller(
        "delivered stale listing released native mount on caller",
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_native_listing_keeps_worker_admission_until_the_native_call_returns() {
    let mut fixture = Fixture::build(true, true);
    let budget = Arc::new(Semaphore::new(1));
    fixture.controller.budget = budget.clone();
    fixture.navigate().await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.entered.try_recv().is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.controller.cancel(PaneId::Left);
    fixture.tasks.abort_all();
    while fixture.tasks.join_next().await.is_some() {}
    assert_eq!(
        budget.available_permits(),
        0,
        "aborting async waiter released native admission"
    );
    fixture.release.take().unwrap().send(()).unwrap();
    fixture.assert_native_cleanup_off_caller("cancelled native result lost guarded ownership");
    tokio::time::timeout(Duration::from_secs(5), async {
        while budget.available_permits() == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn stale_public_archive_command_retires_raw_native_input_before_controller_admission() {
    let mut fixture = Fixture::new(false);
    let (commands, input) = tokio::sync::mpsc::channel(4);
    let (sender, _events) = tokio::sync::mpsc::channel(256);
    let engine = crate::Engine::new(
        crate::Engine::lifetime().unwrap(),
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
        sender,
    );
    let task = tokio::spawn(engine.run(input));
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: fixture._root.path().to_owned(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    commands
        .send(Command::ArchiveNavigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            action: fixture.action.take().unwrap(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let observed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(on) = fixture.dropped.try_recv() {
                break on;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(
        fixture.entered.try_recv().is_err(),
        "stale input reached archive worker"
    );
    assert_ne!(
        observed.unwrap(),
        thread::current().id(),
        "stale dispatch rejected raw native input on executor"
    );
}

#[test]
fn aborting_guarded_engine_run_during_archive_intake_wait_retires_raw_input_off_caller() {
    use std::{
        future::Future,
        task::{Context, Waker},
    };
    let Fixture {
        _root,
        mut controller,
        dropped,
        action,
        ..
    } = Fixture::new(false);
    let occupied: Vec<_> = (0..RETIREMENT_CAPACITY)
        .map(|_| controller.retirement.domain.try_reserve().unwrap())
        .collect();
    let run_domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let mut run = Box::pin(crate::engine_run::EngineRun::new(
        run_domain.try_reserve().unwrap(),
        async move {
            controller.retire_action(action.unwrap()).await.unwrap();
        },
        TaskLifetime::testing(()),
    ));
    assert!(
        run.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert!(dropped.try_recv().is_err());
    // The real EngineRun envelope owns the suspended dispatcher future, including
    // an action that has not acquired a request lease yet. No Tokio runtime exists.
    drop(run);
    let on = dropped.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(occupied);
    assert_ne!(
        on,
        thread::current().id(),
        "cancelled intake exposed raw archive input"
    );
}
