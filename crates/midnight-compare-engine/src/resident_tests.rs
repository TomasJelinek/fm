use super::*;
use crate::policy_test_support::FakePolicySource;
use midnight_core::FileCompareSource;
use midnight_work_budget::resident;
use std::{
    path::Path,
    sync::mpsc::{Receiver, Sender},
    thread::ThreadId,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(10);

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_time()
        .build()
        .unwrap()
}

fn launch(service: &Service, sources: Vec<FileCompareSource>) -> FileCompareLaunch {
    let snapshot = service.comparator_snapshot();
    FileCompareLaunch {
        sources,
        disabled_plugins: Arc::from([]),
        comparators: snapshot.preferences,
        comparator_policy: snapshot.id,
    }
}

fn open(service: &Service, sources: Vec<FileCompareSource>) -> Result<Session, OpenError> {
    service.open(
        launch(service, sources),
        ViewerAppearance {
            family: "monospace".into(),
            font_size: 14.0,
            foreground: [255; 4],
            background: [30, 30, 30, 255],
            selection: [50, 80, 120, 255],
            accent: [90, 150, 220, 255],
        },
        ViewerGeometry {
            width: 320,
            height: 160,
            scale: 1.0,
        },
    )
}

fn wait_for(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "resident comparison did not settle"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn gate(
    spawner: &resident::Spawner,
    handle: &tokio::runtime::Handle,
) -> (Sender<()>, Receiver<ThreadId>) {
    let (entered, arrived) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    spawner
        .spawn(
            handle,
            Box::new(move || {
                entered.send(std::thread::current().id()).unwrap();
                let _ = wait.recv_timeout(TIMEOUT);
            }),
        )
        .ok()
        .unwrap();
    (release, arrived)
}

struct DropProbe {
    dropped: Sender<(ThreadId, usize)>,
    active: Arc<Mutex<Option<Arc<AtomicUsize>>>>,
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        let count = self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .load(Ordering::Acquire);
        let _ = self.dropped.send((std::thread::current().id(), count));
    }
}

#[test]
fn queued_close_cancels_before_capture_and_retires_captures_before_releasing_admission() {
    let runtime = runtime();
    let (owner, spawner) = resident::Owner::start("compare-queued-test", 2).unwrap();
    let (release_first, first) = gate(&spawner, runtime.handle());
    let first = first.recv_timeout(TIMEOUT).unwrap();
    let (release_second, second) = gate(&spawner, runtime.handle());
    let second = second.recv_timeout(TIMEOUT).unwrap();
    let (dropped, observed) = mpsc::channel();
    let active = Arc::new(Mutex::new(None));
    let probe = DropProbe {
        dropped,
        active: active.clone(),
    };
    let captures = Arc::new(AtomicUsize::new(0));
    let capture_calls = captures.clone();
    let capture: NativeCapture = Arc::new(move |_, _| {
        let _keepalive = &probe;
        capture_calls.fetch_add(1, Ordering::AcqRel);
        Err("queued capture must not execute".into())
    });
    let policy = FakePolicySource::new();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_comparator_policies(policy.clone());
    *active.lock().unwrap() = Some(service.active.clone());
    let counter = service.active.clone();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut session = open(
        &service,
        vec![
            FileCompareSource::Focused(root.join("first.txt")),
            FileCompareSource::Focused(root.join("second.txt")),
        ],
    )
    .unwrap();
    assert_eq!(policy.live_pins.load(Ordering::Acquire), 1);
    session.close();
    drop(service);
    assert_eq!(counter.load(Ordering::Acquire), 1);
    assert!(observed.try_recv().is_err());

    release_first.send(()).unwrap();
    release_second.send(()).unwrap();
    let completion = runtime.block_on(async {
        tokio::time::timeout(TIMEOUT, &mut session.worker)
            .await
            .expect("queued comparison ticket did not complete")
    });
    assert_eq!(completion, Err(resident::TaskError::Cancelled));
    let (thread, count_at_drop) = observed.recv_timeout(TIMEOUT).unwrap();
    assert!(thread == first || thread == second);
    assert_eq!(count_at_drop, 1);
    wait_for(|| counter.load(Ordering::Acquire) == 0);
    assert_eq!(policy.live_pins.load(Ordering::Acquire), 0);
    assert_eq!(captures.load(Ordering::Acquire), 0);
    assert!(session.mailbox.take().is_none());
    drop(session);
    runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
}

#[test]
fn dropping_the_final_sessions_and_service_releases_both_idle_bodies_before_seal() {
    let runtime = runtime();
    let (owner, spawner) = resident::Owner::start("compare-idle-drop-test", 2).unwrap();
    let policy = FakePolicySource::new();
    let service = Service::new(runtime.handle().clone(), spawner.clone())
        .with_comparator_policies(policy.clone());
    let first = open(&service, Vec::new()).unwrap();
    let second = open(&service, Vec::new()).unwrap();
    let chooser_thread = |session: &Session| {
        wait_for(|| matches!(session.mailbox.take(), Some(Output::ChooseFiles(_))));
        let (entered, reached) = mpsc::sync_channel(1);
        assert!(session.send(Input::ChooserBarrier(entered)).is_ok());
        reached.recv_timeout(TIMEOUT).unwrap()
    };
    let idle_threads = [chooser_thread(&first), chooser_thread(&second)];
    assert_ne!(idle_threads[0], idle_threads[1]);
    assert_eq!(policy.live_pins.load(Ordering::Acquire), 2);
    let counter = service.active.clone();
    assert_eq!(counter.load(Ordering::Acquire), 2);

    drop(service);
    drop(first);
    drop(second);
    wait_for(|| counter.load(Ordering::Acquire) == 0);
    assert_eq!(policy.live_pins.load(Ordering::Acquire), 0);
    assert_eq!(policy.dropped_pins.load(Ordering::Acquire), 2);

    // Hold the first marker until the second starts to prove both execution
    // slots are free again while the pool still accepts work.
    let (release_first, first) = gate(&spawner, runtime.handle());
    let first = first.recv_timeout(TIMEOUT).unwrap();
    let (release_second, second) = gate(&spawner, runtime.handle());
    let second = second.recv_timeout(TIMEOUT).unwrap();
    assert_ne!(first, second);
    assert!(idle_threads.contains(&first) && idle_threads.contains(&second));
    release_first.send(()).unwrap();
    release_second.send(()).unwrap();

    runtime
        .block_on(async { tokio::time::timeout(TIMEOUT, owner.shutdown(runtime.handle())).await })
        .expect("resident workers did not exit")
        .unwrap();
    drop(spawner);
}

#[test]
fn a_stuck_capture_holds_its_credit_while_the_transient_pool_can_run() {
    let runtime = runtime();
    let (owner, spawner) = resident::Owner::start("compare-capture-test", 2).unwrap();
    let (entered, arrived) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let wait = Mutex::new(wait);
    let capture: NativeCapture = Arc::new(move |_: &Path, _| {
        entered.send(std::thread::current().id()).unwrap();
        let _ = wait.lock().unwrap().recv_timeout(TIMEOUT);
        Err("controlled slow capture".into())
    });
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let session = open(
        &service,
        vec![
            FileCompareSource::Focused(root.join("first.txt")),
            FileCompareSource::Focused(root.join("second.txt")),
        ],
    )
    .unwrap();
    let capture_thread = arrived.recv_timeout(TIMEOUT).unwrap();
    drop(session);
    let other = open(&service, Vec::new()).unwrap();
    assert!(matches!(
        open(&service, Vec::new()),
        Err(OpenError::SessionLimit)
    ));

    let (transient, ran) = mpsc::channel();
    runtime
        .handle()
        .spawn_blocking(move || transient.send(std::thread::current().id()).unwrap());
    assert_ne!(ran.recv_timeout(TIMEOUT).unwrap(), capture_thread);
    assert_eq!(service.active.load(Ordering::Acquire), 2);

    release.send(()).unwrap();
    wait_for(|| service.active.load(Ordering::Acquire) == 1);
    let fresh = open(&service, Vec::new()).unwrap();
    drop(fresh);
    drop(other);
    wait_for(|| service.active.load(Ordering::Acquire) == 0);
    runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
}

#[test]
fn refused_work_keeps_its_credit_until_transient_retirement() {
    let runtime = runtime();
    let (owner, spawner) = resident::Owner::start("compare-refused-test", 2).unwrap();
    let (entered, arrived) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    runtime.handle().spawn_blocking(move || {
        entered.send(std::thread::current().id()).unwrap();
        let _ = wait.recv_timeout(TIMEOUT);
    });
    let transient_thread = arrived.recv_timeout(TIMEOUT).unwrap();
    owner.seal();

    let (dropped, observed) = mpsc::channel();
    let active = Arc::new(Mutex::new(None));
    let probe = DropProbe {
        dropped,
        active: active.clone(),
    };
    let capture: NativeCapture = Arc::new(move |_, _| {
        let _keepalive = &probe;
        panic!("a refused envelope must never run");
    });
    let policy = FakePolicySource::new();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_comparator_policies(policy.clone());
    *active.lock().unwrap() = Some(service.active.clone());
    let counter = service.active.clone();
    assert!(matches!(
        open(&service, Vec::new()),
        Err(OpenError::Unavailable(_))
    ));
    drop(service);
    assert_eq!(counter.load(Ordering::Acquire), 1);
    assert_eq!(policy.live_pins.load(Ordering::Acquire), 1);
    assert!(observed.try_recv().is_err());

    release.send(()).unwrap();
    let (thread, count_at_drop) = observed.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(thread, transient_thread);
    assert_eq!(count_at_drop, 1);
    wait_for(|| counter.load(Ordering::Acquire) == 0);
    assert_eq!(policy.live_pins.load(Ordering::Acquire), 0);
    runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
}

#[test]
fn refusal_after_transient_shutdown_never_drops_work_on_the_caller_or_returns_credit() {
    let runtime = runtime();
    let (owner, spawner) = resident::Owner::start("compare-refused-shutdown-test", 2).unwrap();
    let handle = runtime.handle().clone();
    runtime.block_on(owner.shutdown(&handle)).unwrap();
    drop(runtime);

    let (dropped, observed) = mpsc::channel();
    let active = Arc::new(Mutex::new(None));
    let probe = DropProbe {
        dropped,
        active: active.clone(),
    };
    let capture: NativeCapture = Arc::new(move |_, _| {
        let _keepalive = &probe;
        panic!("a refused envelope must never run");
    });
    let policy = FakePolicySource::new();
    let service = Service::with_native_capture(handle, spawner, capture)
        .with_comparator_policies(policy.clone());
    *active.lock().unwrap() = Some(service.active.clone());
    for _ in 0..2 {
        assert!(matches!(
            open(&service, Vec::new()),
            Err(OpenError::Unavailable(_))
        ));
    }
    assert!(matches!(
        open(&service, Vec::new()),
        Err(OpenError::SessionLimit)
    ));
    let counter = service.active.clone();
    drop(service);
    assert_eq!(counter.load(Ordering::Acquire), 2);
    assert_eq!(policy.live_pins.load(Ordering::Acquire), 2);
    assert!(matches!(
        observed.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
}
