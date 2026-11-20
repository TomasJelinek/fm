use super::*;
use crate::IdentityVerdict;

fn settled(session: &Session) -> Frame {
    wait_for(|last_status| match session.mailbox.take() {
        Some(Output::Frame { frame, .. }) if frame.identity != IdentityVerdict::Pending => {
            Some(frame)
        }
        Some(Output::Status(status)) => {
            *last_status = Some(status);
            None
        }
        Some(Output::ChooseFiles(choice)) => panic!("Unexpected file choice: {choice:?}"),
        Some(Output::ExternalStarted { .. }) => {
            panic!("Built-in fixture launched an external program")
        }
        _ => None,
    })
}

#[test]
fn retry_increments_comparison_generation_but_geometry_and_text_navigation_do_not() {
    let root = fixture();
    let paths = paths(root.path());
    let reads = Arc::new(AtomicUsize::new(0));
    let capture: NativeCapture = Arc::new(move |_, _| memory(b"same\ntext\n", &reads));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    // The held font scan does not answer during the test, so no catalog repaint can be
    // published between the verdict and the frames that answer the inputs below.
    let (service, _catalog) = held_scan(
        Service::with_native_capture(runtime.handle().clone(), spawner, capture),
        &Arc::default(),
    );
    let session = open(&service, launch(&paths)).unwrap();
    let first = frame(&session);
    assert_eq!(first.identity, IdentityVerdict::Pending);
    let initial = settled(&session);
    assert_eq!(initial.identity, IdentityVerdict::IdenticalFiles);
    assert_eq!(initial.comparison_generation, first.comparison_generation);
    assert!(
        session
            .send(Input::Resize(
                ViewerGeometry {
                    width: 400,
                    height: 80,
                    scale: 1.0
                },
                99,
            ))
            .is_ok()
    );
    let resized = frame(&session);
    assert_eq!(resized.geometry_revision, 99);
    assert_eq!(resized.comparison_generation, initial.comparison_generation);
    assert_eq!(resized.identity, IdentityVerdict::IdenticalFiles);
    assert!(session.send(Input::Seek(1.0)).is_ok());
    assert_eq!(
        frame(&session).comparison_generation,
        initial.comparison_generation
    );
    assert!(session.send(Input::Compare(paths)).is_ok());
    let retry = frame(&session);
    assert!(retry.comparison_generation > initial.comparison_generation);
    assert_eq!(retry.identity, IdentityVerdict::Pending);
    let result = settled(&session);
    assert_eq!(result.comparison_generation, retry.comparison_generation);
    assert_eq!(result.identity, IdentityVerdict::IdenticalFiles);
    session.close();
}

#[derive(Debug)]
struct PausedRead {
    reads: AtomicUsize,
    revision: AtomicUsize,
    entered: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl ContentSource for PausedRead {
    fn length(&self) -> u64 {
        5
    }
    fn display_name(&self) -> String {
        "captured.txt".into()
    }
    fn version(&self) -> Vec<u8> {
        self.revision.load(Ordering::Acquire).to_le_bytes().to_vec()
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, amount: u32) -> Result<Vec<u8>, Error> {
        if self.reads.fetch_add(1, Ordering::AcqRel) == 1 {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
        let start = offset.min(5) as usize;
        let end = (start + amount as usize).min(5);
        Ok(b"same\n"[start..end].to_vec())
    }
}

struct PausedSession {
    session: Session,
    _owner: midnight_work_budget::resident::Owner,
    /// Holds the font scan, so neither a catalog repaint nor its wake reaches the worker:
    /// a repaint after the first frame was taken would hold back whole-input verification,
    /// and a queued wake would take an input slot while the worker is paused.
    _catalog: mpsc::Sender<Arc<[String]>>,
    source: Arc<PausedRead>,
    entered: mpsc::Receiver<()>,
    release: Option<mpsc::SyncSender<()>>,
}

impl PausedSession {
    fn open(runtime: &tokio::runtime::Runtime, paths: &[PathBuf; 2]) -> Self {
        let (entered, entry) = mpsc::sync_channel(1);
        let (release, released) = mpsc::sync_channel(1);
        let source = Arc::new(PausedRead {
            reads: AtomicUsize::new(0),
            revision: AtomicUsize::new(1),
            entered,
            release: Mutex::new(released),
        });
        let left = paths[0].clone();
        let captured = source.clone();
        let reads = Arc::new(AtomicUsize::new(0));
        let capture: NativeCapture = Arc::new(move |path, _| {
            if path == left {
                CapturedFile::from_source(captured.clone()).map_err(|error| error.to_string())
            } else {
                memory(b"same\n", &reads)
            }
        });
        let (owner, spawner) = crate::resident_test_support::workers();
        let (service, catalog) = held_scan(
            Service::with_native_capture(runtime.handle().clone(), spawner, capture),
            &Arc::default(),
        );
        Self {
            session: open(&service, launch(paths)).unwrap(),
            _owner: owner,
            _catalog: catalog,
            source,
            entered: entry,
            release: Some(release),
        }
    }

    fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

impl Drop for PausedSession {
    fn drop(&mut self) {
        self.session.close();
        self.release();
    }
}

#[test]
fn initial_frame_precedes_whole_input_work_and_late_proof_cannot_revive_a_replaced_pair() {
    let root = fixture();
    let paths = paths(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut paused = PausedSession::open(&runtime, &paths);
    let initial = frame(&paused.session);
    assert_eq!(initial.identity, IdentityVerdict::Pending);
    paused.entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let replacement = [
        paths[0].with_file_name("replacement-left.txt"),
        paths[1].with_file_name("replacement-right.txt"),
    ];
    assert!(paused.session.send(Input::Compare(replacement)).is_ok());
    assert!(!matches!(
        paused.session.mailbox.take(),
        Some(Output::Frame { frame, .. }) if frame.comparison_generation == initial.comparison_generation
    ));
    paused.release();
    let retry = frame(&paused.session);
    assert!(retry.comparison_generation > initial.comparison_generation);
    assert_eq!(retry.identity, IdentityVerdict::Pending);
    let result = settled(&paused.session);
    assert_eq!(result.comparison_generation, retry.comparison_generation);
    assert_eq!(result.identity, IdentityVerdict::IdenticalFiles);
}

#[test]
fn close_during_whole_input_io_suppresses_late_verdicts() {
    let root = fixture();
    let paths = paths(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut paused = PausedSession::open(&runtime, &paths);
    assert_eq!(frame(&paused.session).identity, IdentityVerdict::Pending);
    paused.entered.recv_timeout(Duration::from_secs(5)).unwrap();
    paused.session.close();
    paused.release();
    wait_for(|_| (paused.source.reads.load(Ordering::Acquire) >= 2).then_some(()));
    assert!(paused.session.mailbox.take().is_none());
}

#[test]
fn source_change_during_whole_input_io_replaces_pending_frame_with_an_error() {
    let root = fixture();
    let paths = paths(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut paused = PausedSession::open(&runtime, &paths);
    assert_eq!(frame(&paused.session).identity, IdentityVerdict::Pending);
    paused.entered.recv_timeout(Duration::from_secs(5)).unwrap();
    paused.source.revision.store(2, Ordering::Release);
    paused.release();
    let error = choice(&paused.session);
    assert!(error.is_error);
    assert_eq!(error.paths, paths);
}

#[test]
fn rejected_retry_on_full_input_queue_preserves_the_active_comparison_generation() {
    let root = fixture();
    let paths = paths(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut paused = PausedSession::open(&runtime, &paths);
    let initial = frame(&paused.session);
    paused.entered.recv_timeout(Duration::from_secs(5)).unwrap();
    for revision in 1..=32 {
        assert!(
            paused
                .session
                .send(Input::Resize(
                    ViewerGeometry {
                        width: 400,
                        height: 80,
                        scale: 1.0
                    },
                    revision,
                ))
                .is_ok()
        );
    }
    assert!(paused.session.send(Input::Compare(paths)).is_err());
    paused.release();
    let result = wait_for(|_| match paused.session.mailbox.take() {
        Some(Output::Frame { frame, .. })
            if frame.identity == IdentityVerdict::IdenticalFiles
                && frame.geometry_revision == 32 =>
        {
            Some(frame)
        }
        Some(Output::ChooseFiles(choice)) => panic!("Unexpected file choice: {choice:?}"),
        Some(Output::ExternalStarted { .. }) => {
            panic!("Built-in fixture launched an external program")
        }
        _ => None,
    });
    assert_eq!(result.comparison_generation, initial.comparison_generation);
    assert_eq!(result.identity, IdentityVerdict::IdenticalFiles);
    assert_eq!(result.geometry_revision, 32);
}

/// A text file whose native Windows revision moves only in its ChangeTime, as when endpoint
/// protection adds an alternate data stream to a freshly written file: during read `during`
/// (counted from zero), or when a test calls `drift`.
#[derive(Debug)]
struct Drifting {
    reads: AtomicUsize,
    during: Option<usize>,
    /// Content and ChangeTime.
    state: Mutex<(&'static [u8], i64)>,
}

impl Drifting {
    fn new(content: &'static [u8], during: Option<usize>) -> Arc<Self> {
        Arc::new(Self {
            reads: AtomicUsize::new(0),
            during,
            state: Mutex::new((content, 0)),
        })
    }

    /// Move the ChangeTime, with `content` of the same length.
    fn drift(&self, content: &'static [u8]) {
        let mut state = self.state.lock().unwrap();
        *state = (content, state.1 + 1);
    }
}

impl ContentSource for Drifting {
    fn length(&self) -> u64 {
        self.state.lock().unwrap().0.len() as u64
    }
    fn display_name(&self) -> String {
        "drifting.txt".into()
    }
    fn version(&self) -> Vec<u8> {
        let (content, change) = *self.state.lock().unwrap();
        // Volume and file ID, length, LastWriteTime and ChangeTime.
        [
            b"native.windows.read.v1".as_slice(),
            &[7; 24],
            &(content.len() as u64).to_le_bytes(),
            &100_i64.to_le_bytes(),
            &change.to_le_bytes(),
        ]
        .concat()
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, amount: u32) -> Result<Vec<u8>, Error> {
        let content = self.state.lock().unwrap().0;
        if Some(self.reads.fetch_add(1, Ordering::AcqRel)) == self.during {
            self.drift(content);
        }
        let start = (offset as usize).min(content.len());
        let end = start.saturating_add(amount as usize).min(content.len());
        Ok(content[start..end].to_vec())
    }
}

/// Compares `left` with an unchanging copy of its text. The held font scan keeps catalog
/// repaints out of the frames the tests expect.
fn drifting_session(
    runtime: &tokio::runtime::Runtime,
    paths: &[PathBuf; 2],
    left: &Arc<Drifting>,
) -> (
    Session,
    midnight_work_budget::resident::Owner,
    mpsc::Sender<Arc<[String]>>,
) {
    let expected = paths[0].clone();
    let captured = left.clone();
    let reads = Arc::new(AtomicUsize::new(0));
    let capture: NativeCapture = Arc::new(move |path, _| {
        if path == expected {
            CapturedFile::from_source(captured.clone()).map_err(|error| error.to_string())
        } else {
            memory(b"same\n", &reads)
        }
    });
    let (owner, spawner) = crate::resident_test_support::workers();
    let (service, catalog) = held_scan(
        Service::with_native_capture(runtime.handle().clone(), spawner, capture),
        &Arc::default(),
    );
    (open(&service, launch(paths)).unwrap(), owner, catalog)
}

#[test]
fn change_time_drift_during_whole_input_io_with_unchanged_text_restarts_verification() {
    let root = fixture();
    let paths = paths(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    // Read 0 is the initial text read; read 1 is the first whole-input read.
    let left = Drifting::new(b"same\n", Some(1));
    let (session, _owner, _catalog) = drifting_session(&runtime, &paths, &left);
    assert_eq!(frame(&session).identity, IdentityVerdict::Pending);
    assert_eq!(settled(&session).identity, IdentityVerdict::IdenticalFiles);
    // The initial read, the interrupted read, one complete read of the fresh capture and
    // the read of the verification restarted on it.
    assert_eq!(left.reads.load(Ordering::Acquire), 4);
    session.close();
}

#[test]
fn change_time_drift_during_the_initial_text_read_reads_the_fresh_capture() {
    let root = fixture();
    let paths = paths(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let left = Drifting::new(b"same\n", Some(0));
    let (session, _owner, _catalog) = drifting_session(&runtime, &paths, &left);
    assert!(frame(&session).accessible_text.contains("same"));
    assert_eq!(settled(&session).identity, IdentityVerdict::IdenticalFiles);
    session.close();
}

#[test]
fn a_repaint_after_change_time_drift_keeps_unchanged_text_and_reports_changed_text() {
    let root = fixture();
    let paths = paths(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let left = Drifting::new(b"same\n", None);
    let (session, _owner, _catalog) = drifting_session(&runtime, &paths, &left);
    assert_eq!(frame(&session).identity, IdentityVerdict::Pending);
    assert_eq!(settled(&session).identity, IdentityVerdict::IdenticalFiles);
    let geometry = ViewerGeometry {
        width: 400,
        height: 80,
        scale: 1.0,
    };
    left.drift(b"same\n");
    assert!(session.send(Input::Resize(geometry, 7)).is_ok());
    let resized = frame(&session);
    assert_eq!(resized.geometry_revision, 7);
    assert_eq!(resized.identity, IdentityVerdict::IdenticalFiles);
    // Same length and LastWriteTime: only the content shows that this file changed.
    left.drift(b"SAME\n");
    assert!(session.send(Input::Resize(geometry, 8)).is_ok());
    let error = choice(&session);
    assert!(error.is_error);
    assert!(
        error.message.starts_with("Compared file changed: "),
        "{}",
        error.message
    );
    session.close();
}
