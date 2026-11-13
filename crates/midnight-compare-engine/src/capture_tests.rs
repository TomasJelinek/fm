use crate::{FileChoice, Frame, Input, NativeCapture, Output, Service, Session};
use midnight_core::{
    DirectoryIndex, Entry, EntryId, EntryKind, FileCompareLaunch, FileCompareSource,
    ViewerAppearance, ViewerGeometry,
};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Debug)]
struct Memory {
    bytes: &'static [u8],
    reads: Arc<AtomicUsize>,
}

impl ContentSource for Memory {
    fn length(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn display_name(&self) -> String {
        "provider.txt".into()
    }

    fn version(&self) -> Vec<u8> {
        vec![1]
    }

    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }

    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let start = offset.min(self.length()) as usize;
        let end = start.saturating_add(length as usize).min(self.bytes.len());
        Ok(self.bytes[start..end].to_vec())
    }
}

fn memory(bytes: &'static [u8], reads: &Arc<AtomicUsize>) -> Result<CapturedFile, String> {
    CapturedFile::from_source(Arc::new(Memory {
        bytes,
        reads: reads.clone(),
    }))
    .map_err(|error| error.to_string())
}

fn fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("_build/compare-capture-tests");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

fn paths(root: &Path) -> [PathBuf; 2] {
    [
        root.join("left").join("Žluťoučký 世界 🐈.txt"),
        root.join("right").join("Žluťoučký 世界 🐈.txt"),
    ]
}

fn launch(paths: &[PathBuf; 2]) -> FileCompareLaunch {
    FileCompareLaunch {
        sources: paths
            .iter()
            .cloned()
            .map(FileCompareSource::Focused)
            .collect(),
        disabled_plugins: Arc::from([]),
    }
}

fn unresolved() -> FileCompareLaunch {
    FileCompareLaunch {
        sources: Vec::new(),
        disabled_plugins: Arc::from([]),
    }
}

fn open(service: &Service, launch: FileCompareLaunch) -> Result<Session, String> {
    service.open(
        launch,
        ViewerAppearance {
            family: "monospace".into(),
            font_size: 14.0,
            foreground: [255; 4],
            background: [30, 30, 30, 255],
            selection: [50, 80, 120, 255],
            accent: [90, 150, 220, 255],
        },
        ViewerGeometry {
            width: 640,
            height: 200,
            scale: 1.0,
        },
    )
}

fn wait_for<T>(mut poll: impl FnMut(&mut Option<String>) -> Option<T>) -> T {
    let start = Instant::now();
    let mut last_status = None;
    loop {
        if let Some(value) = poll(&mut last_status) {
            return value;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "Worker timed out after {:?}; last status: {last_status:?}",
            start.elapsed()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn frame(session: &Session) -> Frame {
    eprintln!(
        "Comparator frame waiter test={:?} mailbox={:p}",
        std::thread::current().name(),
        session.mailbox.as_ref()
    );
    wait_for(|last_status| match session.mailbox.take() {
        Some(Output::Frame { frame, .. }) => Some(frame),
        Some(Output::ChooseFiles(choice)) => panic!("Unexpected file choice: {choice:?}"),
        Some(Output::Status(status)) => {
            *last_status = Some(status);
            None
        }
        None => None,
    })
}

fn choice(session: &Session) -> FileChoice {
    eprintln!(
        "Comparator choice waiter test={:?} mailbox={:p}",
        std::thread::current().name(),
        session.mailbox.as_ref()
    );
    wait_for(|last_status| match session.mailbox.take() {
        Some(Output::ChooseFiles(choice)) => Some(choice),
        Some(Output::Frame { .. }) => panic!("Capture failure must not produce a frame"),
        Some(Output::Status(status)) => {
            *last_status = Some(status);
            None
        }
        None => None,
    })
}

#[test]
fn injected_capture_compares_both_panel_sources_without_opening_native_paths() {
    let root = fixture();
    let paths = paths(root.path());
    assert!(paths.iter().all(|path| !path.exists()));
    let reads = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let record = calls.clone();
    let observed_reads = reads.clone();
    let expected = paths.clone();
    let capture: NativeCapture = Arc::new(move |path, cancel| {
        assert!(!cancel.load(Ordering::Acquire));
        record.lock().unwrap().push((
            path.to_path_buf(),
            std::thread::current().id(),
            std::ptr::from_ref(cancel).addr(),
        ));
        if path == expected[0] {
            memory("provider český\nold\n".as_bytes(), &observed_reads)
        } else if path == expected[1] {
            memory("provider český\nnew\n".as_bytes(), &observed_reads)
        } else {
            Err("Unexpected provider identity".into())
        }
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = Service::with_native_capture(runtime.handle().clone(), capture);
    let entries: Arc<[Entry]> =
        vec![Entry::new(EntryId(7), paths[1].clone(), EntryKind::File)].into();
    let mut launch = launch(&paths);
    launch.sources[1] = FileCompareSource::SameName {
        directory: paths[1].parent().unwrap().to_path_buf(),
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let session = open(&service, launch).unwrap();
    let frame = frame(&session);
    assert!(frame.accessible_text.contains("provider český"));
    assert!(frame.accessible_text.contains("old"));
    assert!(frame.accessible_text.contains("new"));
    assert!(frame.status.contains("1 difference"));
    assert!(reads.load(Ordering::Relaxed) >= 2);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    for (side, (path, thread, cancel)) in calls.iter().enumerate() {
        assert_eq!(path, &paths[side]);
        assert_ne!(*thread, std::thread::current().id());
        assert_eq!(*cancel, Arc::as_ptr(&session.cancelled).addr());
    }
    session.close();
}

#[test]
fn explicit_compare_submission_uses_the_injected_capture_for_both_choices() {
    let root = fixture();
    let paths = paths(root.path());
    let expected = paths.clone();
    let reads = Arc::new(AtomicUsize::new(0));
    let capture: NativeCapture = Arc::new(move |path, _| {
        if path == expected[0] {
            memory(b"explicit first\n", &reads)
        } else if path == expected[1] {
            memory(b"explicit second\n", &reads)
        } else {
            Err("Unexpected provider identity".into())
        }
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = Service::with_native_capture(runtime.handle().clone(), capture);
    let session = open(&service, unresolved()).unwrap();
    assert!(!choice(&session).is_error);
    assert!(session.send(Input::Compare(paths)));
    let frame = frame(&session);
    assert!(frame.accessible_text.contains("explicit first"));
    assert!(frame.accessible_text.contains("explicit second"));
    session.close();
}

#[test]
fn injected_rejection_on_either_side_never_falls_back_to_readable_native_files() {
    let root = fixture();
    let paths = paths(root.path());
    for path in &paths {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"native bytes must not be read\n").unwrap();
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for rejected in 0..2 {
        let rejected_path = paths[rejected].clone();
        let reads = Arc::new(AtomicUsize::new(0));
        let capture: NativeCapture = Arc::new(move |path, _| {
            if path == rejected_path {
                Err("Installed provider unavailable".into())
            } else {
                memory(b"provider bytes\n", &reads)
            }
        });
        let service = Service::with_native_capture(runtime.handle().clone(), capture);
        let session = open(&service, launch(&paths)).unwrap();
        let choice = choice(&session);
        assert!(choice.is_error);
        assert_eq!(choice.paths, paths);
        assert!(choice.message.contains("Installed provider unavailable"));
        assert!(
            choice
                .message
                .contains(if rejected == 0 { "File A" } else { "File B" })
        );
        session.close();
    }
}

#[test]
fn close_cancels_either_capture_retains_capacity_until_return_and_suppresses_late_output() {
    let root = fixture();
    let paths = paths(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for blocked in 0..2 {
        let blocked_path = paths[blocked].clone();
        let reads = Arc::new(AtomicUsize::new(0));
        let observed_reads = reads.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed_calls = calls.clone();
        let (entered, entry) = mpsc::sync_channel(1);
        let (observed, cancellation) = mpsc::sync_channel(1);
        let (release, returned) = mpsc::sync_channel(1);
        let returned = Mutex::new(returned);
        let capture: NativeCapture = Arc::new(move |path, cancel| {
            observed_calls.fetch_add(1, Ordering::Relaxed);
            if path == blocked_path {
                entered.send(std::ptr::from_ref(cancel).addr()).unwrap();
                wait_for(|_| cancel.load(Ordering::Acquire).then_some(()));
                observed.send(()).unwrap();
                returned
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
            }
            memory(b"late provider content\n", &observed_reads)
        });
        let service = Service::with_native_capture(runtime.handle().clone(), capture);
        let session = open(&service, launch(&paths)).unwrap();
        assert_eq!(
            entry.recv_timeout(Duration::from_secs(5)).unwrap(),
            Arc::as_ptr(&session.cancelled).addr()
        );
        session.mailbox.take();
        session.close();
        cancellation.recv_timeout(Duration::from_secs(5)).unwrap();
        let other = open(&service, unresolved()).unwrap();
        assert!(
            open(&service, unresolved()).is_err(),
            "Capture still owns its slot"
        );
        release.send(()).unwrap();
        let next = wait_for(|_| open(&service, unresolved()).ok());
        assert_eq!(calls.load(Ordering::Relaxed), blocked + 1);
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        assert!(
            session.mailbox.take().is_none(),
            "Closed session published late output"
        );
        other.close();
        next.close();
    }
}
