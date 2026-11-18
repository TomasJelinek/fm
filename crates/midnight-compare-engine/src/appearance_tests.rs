use super::*;
use midnight_core::FileCompareSource;
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(10);
const GEOMETRY: ViewerGeometry = ViewerGeometry {
    width: 320,
    height: 80,
    scale: 1.0,
};
const TEXT: &[u8] = b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";

fn original() -> ViewerAppearance {
    ViewerAppearance {
        family: "monospace".into(),
        font_size: 14.0,
        foreground: [255; 4],
        background: [30, 30, 30, 255],
        selection: [50, 80, 120, 255],
        accent: [90, 150, 220, 255],
    }
}

pub(crate) fn changed() -> ViewerAppearance {
    ViewerAppearance {
        family: "ignored application font".into(),
        font_size: 52.0,
        foreground: [220, 235, 250, 255],
        background: [12, 82, 162, 255],
        selection: [100, 20, 90, 255],
        accent: [60, 180, 40, 255],
    }
}

type Gate = (SyncSender<()>, Receiver<()>);

#[derive(Debug, Default)]
struct TextSource {
    gate: Mutex<Option<Gate>>,
}

impl ContentSource for TextSource {
    fn length(&self) -> u64 {
        TEXT.len() as u64
    }

    fn display_name(&self) -> String {
        "captured.txt".into()
    }

    fn version(&self) -> Vec<u8> {
        vec![1]
    }

    fn validate(&self) -> Result<(), Error> {
        let gate = self.gate.lock().unwrap().take();
        if let Some((entered, resume)) = gate {
            entered.send(()).unwrap();
            // Dropping the test sender on assertion failure releases the worker.
            let _ = resume.recv_timeout(TIMEOUT);
        }
        Ok(())
    }

    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        let start = (offset as usize).min(TEXT.len());
        let end = (start + length as usize).min(TEXT.len());
        Ok(TEXT[start..end].to_vec())
    }
}

fn paths() -> [PathBuf; 2] {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    [root.join("palette-a.txt"), root.join("palette-b.txt")]
}

fn open(service: &Service, sources: Vec<FileCompareSource>) -> Session {
    let snapshot = service.comparator_snapshot();
    service
        .open(
            FileCompareLaunch {
                sources,
                disabled_plugins: Arc::from([]),
                comparators: snapshot.preferences,
                comparator_policy: snapshot.id,
            },
            original(),
            GEOMETRY,
        )
        .unwrap()
}

fn service(
    runtime: &tokio::runtime::Runtime,
    spawner: midnight_work_budget::resident::Spawner,
    source: Arc<TextSource>,
    count: Arc<AtomicUsize>,
) -> Service {
    Service::with_native_capture(
        runtime.handle().clone(),
        spawner,
        Arc::new(move |_, _| {
            count.fetch_add(1, Ordering::AcqRel);
            CapturedFile::from_source(source.clone()).map_err(|error| error.to_string())
        }),
    )
    .with_font_scan(|_| Some(Arc::from(["monospace".to_owned()])))
}

fn wait<T>(mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(value) = poll() {
            return value;
        }
        assert!(Instant::now() < deadline, "comparison did not converge");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn next_frame(session: &Session, revision: u64, settled: bool) -> Frame {
    wait(|| match session.mailbox.take() {
        Some(Output::Frame {
            frame,
            appearance_revision,
            ..
        }) => {
            assert_eq!(
                appearance_revision, revision,
                "a stale frame escaped publication"
            );
            (!settled || (frame.identity != IdentityVerdict::Pending && frame.fonts.is_some()))
                .then_some(frame)
        }
        Some(Output::ChooseFiles(choice)) => panic!("unexpected chooser: {choice:?}"),
        Some(Output::ExternalStarted { .. }) => panic!("unexpected external launch"),
        _ => None,
    })
}

#[test]
fn palette_revision_survives_close_while_the_worker_is_still_connected() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = Arc::new(TextSource::default());
    let count = Arc::new(AtomicUsize::new(0));
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = service(&runtime, spawner, source.clone(), count);
    let session = open(&service, Vec::new());
    assert_eq!(session.palette_revision(), 0);
    wait(|| matches!(session.mailbox.take(), Some(Output::ChooseFiles(_))).then_some(()));

    let (entered, reached) = mpsc::sync_channel(1);
    let (resume, gate) = mpsc::sync_channel(1);
    *source.gate.lock().unwrap() = Some((entered, gate));
    assert!(session.send(Input::Compare(paths())).is_ok());
    reached.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(session.restore_palette(&changed()).unwrap(), 1);

    session.close();
    assert!(session.mailbox.state.lock().unwrap().worker_connected);
    assert_eq!(session.palette_revision(), 1);
    assert!(session.restore_palette(&original()).is_err());
    assert_eq!(session.palette_revision(), 1);
    resume.send(()).unwrap();
}

#[test]
fn an_in_flight_old_paint_cannot_beat_the_latest_palette_with_all_input_slots_full() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = Arc::new(TextSource::default());
    let count = Arc::new(AtomicUsize::new(0));
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = service(&runtime, spawner, source.clone(), count.clone());
    let session = open(
        &service,
        paths()
            .into_iter()
            .map(FileCompareSource::Focused)
            .collect(),
    );
    let initial = next_frame(&session, 0, true);
    assert_eq!(initial.identity, IdentityVerdict::IdenticalFiles);

    let (entered, reached) = mpsc::sync_channel(1);
    let (resume, gate) = mpsc::sync_channel(1);
    *source.gate.lock().unwrap() = Some((entered, gate));
    assert!(session.send(Input::Resize(GEOMETRY, 9)).is_ok());
    reached.recv_timeout(TIMEOUT).unwrap();
    for _ in 0..32 {
        assert!(session.send(Input::Seek(0.5)).is_ok());
    }
    assert!(session.send(Input::Seek(0.0)).is_err());

    let current = session.restore_palette(&changed()).unwrap();
    assert_eq!(current, 1);
    let mut typography_only = changed();
    typography_only.family = "different app family".into();
    typography_only.font_size = 72.0;
    assert_eq!(session.restore_palette(&typography_only).unwrap(), current);
    resume.send(()).unwrap();
    let frame = next_frame(&session, current, true);
    assert_eq!(frame.comparison_generation, initial.comparison_generation);
    assert_eq!(frame.identity, initial.identity);
    assert_eq!(frame.geometry_revision, 9);
    assert_eq!(frame.position, 3.0 / 7.0);
    // The 80px viewport at the opening 14px size fits four comparison rows.
    // The application font size in the palette request would fit only one.
    assert_eq!(frame.accessible_text.lines().count(), 4);
    assert!(frame.accessible_text.contains("seven"));
    assert_eq!(
        frame.fonts.unwrap().selected,
        initial.fonts.unwrap().selected
    );
    assert!(
        frame
            .pixels
            .as_chunks::<4>()
            .0
            .contains(&[12, 82, 162, 255])
    );
    assert_eq!(count.load(Ordering::Acquire), 2);

    // Once position has settled, changing only the palette preserves the
    // visible text, document font and navigation geometry.
    let revision = session.restore_palette(&original()).unwrap();
    let restored = next_frame(&session, revision, true);
    assert_eq!(restored.position, frame.position);
    assert_eq!(restored.accessible_text, frame.accessible_text);
    assert_eq!(restored.geometry_revision, frame.geometry_revision);
    assert_eq!(restored.comparison_generation, frame.comparison_generation);
    assert_eq!(restored.identity, frame.identity);
    assert_eq!(count.load(Ordering::Acquire), 2);
}

#[test]
fn first_frame_of_an_in_progress_replacement_uses_the_newest_palette() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = Arc::new(TextSource::default());
    let count = Arc::new(AtomicUsize::new(0));
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = service(&runtime, spawner, source.clone(), count.clone());
    let session = open(&service, Vec::new());
    wait(|| matches!(session.mailbox.take(), Some(Output::ChooseFiles(_))).then_some(()));
    let chooser_generation = session.mailbox.state.lock().unwrap().generation;
    let (entered, reached) = mpsc::sync_channel(1);
    let (resume, gate) = mpsc::sync_channel(1);
    *source.gate.lock().unwrap() = Some((entered, gate));
    assert!(session.send(Input::Compare(paths())).is_ok());
    reached.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(count.load(Ordering::Acquire), 1);

    let mut newest = changed();
    assert_eq!(session.restore_palette(&newest).unwrap(), 1);
    newest.background = [71, 19, 39, 255];
    let latest = session.restore_palette(&newest).unwrap();
    assert_eq!(latest, 2);
    resume.send(()).unwrap();
    let frame = next_frame(&session, latest, false);
    assert!(frame.comparison_generation > chooser_generation);
    assert!(frame.pixels.as_chunks::<4>().0.contains(&[71, 19, 39, 255]));
    assert_eq!(count.load(Ordering::Acquire), 2);
}

#[test]
fn a_to_b_to_a_republishes_a_when_b_never_paints() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = Arc::new(TextSource::default());
    let count = Arc::new(AtomicUsize::new(0));
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = service(&runtime, spawner, source.clone(), count.clone());
    let session = open(
        &service,
        paths()
            .into_iter()
            .map(FileCompareSource::Focused)
            .collect(),
    );
    let initial = next_frame(&session, 0, true);

    let (entered, reached) = mpsc::sync_channel(1);
    let (resume, gate) = mpsc::sync_channel(1);
    *source.gate.lock().unwrap() = Some((entered, gate));
    assert!(session.send(Input::Resize(GEOMETRY, 1)).is_ok());
    reached.recv_timeout(TIMEOUT).unwrap();

    assert_eq!(session.restore_palette(&changed()).unwrap(), 1);
    assert_eq!(session.restore_palette(&original()).unwrap(), 2);
    resume.send(()).unwrap();
    let restored = next_frame(&session, 2, true);
    assert_eq!(restored.pixels, initial.pixels);
    assert_eq!(restored.accessible_text, initial.accessible_text);
    assert_eq!(restored.identity, initial.identity);
    assert_eq!(
        restored.comparison_generation,
        initial.comparison_generation
    );
    assert_eq!(count.load(Ordering::Acquire), 2);
}
