//! Any number of comparisons open; at most `computing::COMPUTING` prepare at once, and
//! the others wait, shown as waiting, until one finishes or they are closed.
use super::*;
use midnight_core::FileCompareSource;
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    path::Path,
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(10);
/// Longer than any wait for output, so a wait that a hold blocks fails first; dropping
/// the release ends a hold at once.
const HOLD: Duration = Duration::from_secs(60);

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

/// Empty content whose validation after its read (the first after its capture) reports
/// its entry and holds its comparison in preparation until a release (or the end of the
/// test) lets it fail. Empty content takes no shared I/O admission, so a held comparison
/// holds back no other test's reads.
#[derive(Debug)]
struct Held {
    validations: AtomicUsize,
    entered: mpsc::Sender<()>,
    release: Arc<Mutex<Receiver<()>>>,
}

impl ContentSource for Held {
    fn length(&self) -> u64 {
        0
    }
    fn display_name(&self) -> String {
        "held.txt".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        match self.validations.fetch_add(1, Ordering::AcqRel) {
            0 => Ok(()),
            1 => {
                let _ = self.entered.send(());
                let _ = self.release.lock().unwrap().recv_timeout(HOLD);
                Err(Error("held read".into()))
            }
            _ => Err(Error("held read".into())),
        }
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
        Ok(Vec::new())
    }
}

/// Text that reads at once.
#[derive(Debug)]
struct Text;

impl ContentSource for Text {
    fn length(&self) -> u64 {
        5
    }
    fn display_name(&self) -> String {
        "text.txt".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        let bytes = b"text\n";
        let start = (offset as usize).min(bytes.len());
        let end = start.saturating_add(length as usize).min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }
}

/// A capture whose files open at once and whose read holds its comparison in preparation:
/// an admitted comparison reports one entry.
fn held_capture() -> (NativeCapture, Receiver<()>, mpsc::Sender<()>) {
    let (entered, arrived) = mpsc::channel();
    let (release, wait) = mpsc::channel::<()>();
    let release_reads = Arc::new(Mutex::new(wait));
    let entered = Mutex::new(entered);
    let capture: NativeCapture = Arc::new(move |_: &Path, _| {
        let held = Held {
            validations: AtomicUsize::new(0),
            entered: entered.lock().unwrap().clone(),
            release: release_reads.clone(),
        };
        CapturedFile::from_source(Arc::new(held)).map_err(|error| error.to_string())
    });
    (capture, arrived, release)
}

fn quiet(service: Service) -> Service {
    service
        .with_image_viewer_discovery(|| Ok(Vec::new()))
        .with_font_scan(|_| Some(Arc::from([])))
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
    let service = quiet(Service::with_native_capture(
        runtime.handle().clone(),
        spawner,
        capture,
    ))
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
        "the waiting comparison reads no file"
    );
    // One preparation ends (its read fails): the waiting one starts.
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

/// Closing a comparison that waits ends its worker without reading a file.
#[test]
fn closing_a_waiting_comparison_ends_it_without_preparing() {
    let runtime = runtime();
    let (owner, spawner) = crate::resident_test_support::workers();
    let (capture, entered, release) = held_capture();
    let service = quiet(Service::with_native_capture(
        runtime.handle().clone(),
        spawner,
        capture,
    ))
    .with_computing(1);
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
        "the closed comparison never read a file"
    );
    first.close();
    drop((first, waiting_session, service));
    runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
}

/// An open that blocks (an unresponsive share) holds no permit: with one permit, another
/// comparison is prepared and presented meanwhile.
#[test]
fn an_open_that_blocks_holds_no_permit() {
    let runtime = runtime();
    let (owner, spawner) = crate::resident_test_support::workers();
    let (entered, arrived) = mpsc::channel();
    let (release, wait) = mpsc::channel::<()>();
    let wait = Mutex::new(wait);
    let entered = Mutex::new(entered);
    let opens = AtomicUsize::new(0);
    let capture: NativeCapture = Arc::new(move |_: &Path, _| {
        if opens.fetch_add(1, Ordering::AcqRel) == 0 {
            entered.lock().unwrap().send(()).unwrap();
            let _ = wait.lock().unwrap().recv_timeout(HOLD);
            return Err("unresponsive share".into());
        }
        CapturedFile::from_source(Arc::new(Text)).map_err(|error| error.to_string())
    });
    let service = quiet(Service::with_native_capture(
        runtime.handle().clone(),
        spawner,
        capture,
    ))
    .with_computing(1);
    let stuck = open(&service);
    arrived.recv_timeout(TIMEOUT).expect("the stuck open");
    let other = open(&service);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match other.mailbox.take() {
            Some(Output::Frame { .. }) => break,
            Some(Output::ChooseFiles(choice)) => panic!("{}", choice.message),
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "the other comparison waits for the stuck open"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    drop(release);
    for session in [&stuck, &other] {
        session.close();
    }
    drop((stuck, other, service));
    runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
}

/// A replacement sent to a waiting comparison ends that wait: the replacement opens its
/// files and waits in its own turn, while the admitted comparison still holds the permit.
#[test]
fn a_replacement_ends_the_wait_of_the_comparison_it_replaces() {
    let runtime = runtime();
    let (owner, spawner) = crate::resident_test_support::workers();
    let (capture, entered, release) = held_capture();
    let service = quiet(Service::with_native_capture(
        runtime.handle().clone(),
        spawner,
        capture,
    ))
    .with_computing(1);
    let first = open(&service);
    entered.recv_timeout(TIMEOUT).unwrap();
    let second = open(&service);
    assert!(waiting(&second));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    second
        .send(Input::Compare([
            root.join("third.txt"),
            root.join("fourth.txt"),
        ]))
        .unwrap();
    assert!(waiting(&second), "the replacement waits in its own turn");
    assert!(
        entered.recv_timeout(Duration::from_millis(200)).is_err(),
        "the replacement reads no file before a permit"
    );
    drop(release);
    for session in [&first, &second] {
        session.close();
    }
    drop((first, second, service));
    runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
}

/// A failing waiting status (its mailbox drops outputs and wakes the UI) unwinds out of
/// `acquire`; its place in the queue must go with it, or every later comparison waits.
#[test]
fn a_waiter_whose_status_panics_leaves_the_queue() {
    let gate = Arc::new(computing::Gate::new(1));
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let admitted = gate.acquire(&cancel, || false, || {}).unwrap();
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gate.acquire(&cancel, || false, || panic!("the waiting status failed"))
    }));
    assert!(unwound.is_err());
    drop(admitted);
    let (sender, receiver) = mpsc::channel();
    let next = gate.clone();
    std::thread::spawn(move || {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let admitted = next.acquire(&cancel, || false, || {});
        let _ = sender.send(admitted.is_ok_and(|permit| permit.is_some()));
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
