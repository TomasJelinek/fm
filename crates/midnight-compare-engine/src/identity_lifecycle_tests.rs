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
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture);
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
        let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture);
        Self {
            session: open(&service, launch(paths)).unwrap(),
            _owner: owner,
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
