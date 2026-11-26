//! Any number of comparisons open; at most `computing::COMPUTING` prepare at once, and
//! the others wait, shown as waiting, until one finishes or they are closed.
use super::*;
use midnight_core::FileCompareSource;
use std::{
    path::Path,
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(10);

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .unwrap()
}

fn open(service: &Service) -> Session {
    let snapshot = service.comparator_snapshot();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    service
        .open(
            FileCompareLaunch {
                sources: vec![
                    FileCompareSource::Focused(root.join("first.txt")),
                    FileCompareSource::Focused(root.join("second.txt")),
                ],
                disabled_plugins: Arc::from([]),
                comparators: snapshot.preferences,
                comparator_policy: snapshot.id,
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [255; 4],
                background: [30, 30, 30, 255],
                selection: [50, 80, 120, 255],
                selection_text: [0; 4],
                accent: [90, 150, 220, 255],
            },
            ViewerGeometry {
                width: 320,
                height: 160,
                scale: 1.0,
            },
        )
        .expect("sessions are not limited")
}

/// A capture that reports its entry and holds its comparison in preparation until a
/// release (or the end of the test) lets it fail.
fn held_capture() -> (NativeCapture, Receiver<()>, mpsc::Sender<()>) {
    let (entered, arrived) = mpsc::channel();
    let (release, wait) = mpsc::channel::<()>();
    let wait = Mutex::new(wait);
    let entered = Mutex::new(entered);
    let capture: NativeCapture = Arc::new(move |_: &Path, _| {
        entered.lock().unwrap().send(()).unwrap();
        let _ = wait.lock().unwrap().recv_timeout(TIMEOUT);
        Err("held capture".into())
    });
    (capture, arrived, release)
}

fn waiting(session: &Session) -> bool {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match session.mailbox.take() {
            Some(Output::Status(status)) if status == computing::WAITING => return true,
            Some(Output::Status(_)) | None => {}
            Some(_) => return false,
        }
        assert!(Instant::now() < deadline, "no waiting status");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// With `permits` preparations admitted, one comparison more waits, shown as waiting,
/// and starts as soon as one of them finishes.
fn one_more_waits_and_starts_when_one_finishes(permits: usize) {
    let runtime = runtime();
    let (owner, spawner) = crate::resident_test_support::workers();
    let (capture, entered, release) = held_capture();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_computing(permits);
    let sessions: Vec<_> = (0..permits).map(|_| open(&service)).collect();
    for _ in 0..permits {
        entered
            .recv_timeout(TIMEOUT)
            .expect("an admitted preparation");
    }
    let last = open(&service);
    assert!(waiting(&last), "comparison {} waits", permits + 1);
    assert!(
        entered.recv_timeout(Duration::from_millis(200)).is_err(),
        "the waiting comparison opens no file"
    );
    // One preparation ends (its capture fails): the waiting one starts.
    release.send(()).unwrap();
    entered
        .recv_timeout(TIMEOUT)
        .expect("the waiting comparison starts once one finishes");
    drop(release);
    for session in sessions.iter().chain([&last]) {
        session.close();
    }
    drop((sessions, last, service));
    runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
}

#[test]
fn the_comparison_after_the_admitted_preparations_waits_for_one_to_finish() {
    one_more_waits_and_starts_when_one_finishes(computing::COMPUTING);
}

/// The test the owner's decision asked for: six comparisons prepare, the seventh waits.
#[test]
fn with_six_admitted_the_seventh_waits_and_runs_when_one_finishes() {
    one_more_waits_and_starts_when_one_finishes(6);
}

/// Closing a comparison that waits ends its worker without opening a file.
#[test]
fn closing_a_waiting_comparison_ends_it_without_preparing() {
    let runtime = runtime();
    let (owner, spawner) = crate::resident_test_support::workers();
    let (capture, entered, release) = held_capture();
    let service =
        Service::with_native_capture(runtime.handle().clone(), spawner, capture).with_computing(1);
    let first = open(&service);
    entered.recv_timeout(TIMEOUT).unwrap();
    let mut waiting_session = open(&service);
    assert!(waiting(&waiting_session));
    waiting_session.close();
    let finished = runtime.block_on(async {
        tokio::time::timeout(TIMEOUT, &mut waiting_session.worker)
            .await
            .expect("the closed waiting comparison ended")
    });
    assert_eq!(finished, Ok(()));
    drop(release);
    assert!(
        entered.recv_timeout(Duration::from_millis(200)).is_err(),
        "the closed comparison never opened a file"
    );
    first.close();
    drop((first, waiting_session, service));
    runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
}

/// A failing waiting status (its mailbox drops outputs and wakes the UI) unwinds out of
/// `acquire`; its place in the queue must go with it, or every later comparison waits.
#[test]
fn a_waiter_whose_status_panics_leaves_the_queue() {
    let gate = Arc::new(computing::Gate::new(1));
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let admitted = gate.acquire(&cancel, || {}).unwrap();
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gate.acquire(&cancel, || panic!("the waiting status failed"))
    }));
    assert!(unwound.is_err());
    drop(admitted);
    let (sender, receiver) = mpsc::channel();
    let next = gate.clone();
    std::thread::spawn(move || {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let _ = sender.send(next.acquire(&cancel, || {}).is_ok());
    });
    assert_eq!(
        receiver.recv_timeout(TIMEOUT),
        Ok(true),
        "the next comparison is admitted"
    );
}

/// Six comparisons open at once, each with its own worker; none is refused.
#[test]
fn six_comparisons_open_at_once() {
    let runtime = runtime();
    let (owner, spawner) = crate::resident_test_support::workers();
    let service = Service::new(runtime.handle().clone(), spawner);
    let sessions: Vec<_> = (0..6).map(|_| open(&service)).collect();
    // Each reaches its worker: its files are missing, so each asks for them.
    for session in &sessions {
        let deadline = Instant::now() + TIMEOUT;
        while !matches!(session.mailbox.take(), Some(Output::ChooseFiles(_))) {
            assert!(Instant::now() < deadline, "a comparison never started");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    assert_eq!(service.active.load(Ordering::Acquire), 6);
    for session in &sessions {
        session.close();
    }
    drop(sessions);
    runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
}
