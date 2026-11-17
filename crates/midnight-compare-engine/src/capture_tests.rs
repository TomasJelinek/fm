use crate::{FileChoice, Frame, Input, NativeCapture, Output, Refused, Service, Session};
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
    open_in_family(service, launch, "monospace")
}

fn open_in_family(
    service: &Service,
    launch: FileCompareLaunch,
    family: &str,
) -> Result<Session, String> {
    service.open(
        launch,
        ViewerAppearance {
            family: family.into(),
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
fn storage_hints_are_queried_on_the_worker_and_viewers_are_discovered_once_per_service() {
    let root = fixture();
    let paths = paths(root.path());
    let reads = Arc::new(AtomicUsize::new(0));
    let expected = paths.clone();
    let capture: NativeCapture = Arc::new(move |path, _| {
        if path == expected[0] {
            memory(b"same\nold\n", &reads)
        } else {
            memory(b"same\nnew\n", &reads)
        }
    });
    let queried = Arc::new(Mutex::new(Vec::new()));
    let record = queried.clone();
    let caller = std::thread::current().id();
    let discoveries = Arc::new(AtomicUsize::new(0));
    let discovered_by = discoveries.clone();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = Service::with_native_capture(runtime.handle().clone(), capture)
        .with_storage_hint(Arc::new(move |path| {
            assert_ne!(
                std::thread::current().id(),
                caller,
                "storage queries are backend work"
            );
            record.lock().unwrap().push(path.to_path_buf());
            true
        }))
        .with_image_viewer_discovery(move || {
            assert_ne!(
                std::thread::current().id(),
                caller,
                "discovery is backend work"
            );
            discovered_by.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        });
    let first = open(&service, launch(&paths)).unwrap();
    assert!(frame(&first).status.contains("1 difference group"));
    let discovered = service
        .viewers
        .kept()
        .expect("the first comparison discovers installed image viewers");
    let second = open(&service, launch(&paths)).unwrap();
    frame(&second);
    assert!(Arc::ptr_eq(&discovered, &service.viewers.kept().unwrap()));
    assert_eq!(discoveries.load(Ordering::SeqCst), 1);
    assert_eq!(
        *queried.lock().unwrap(),
        [
            paths[0].clone(),
            paths[1].clone(),
            paths[0].clone(),
            paths[1].clone()
        ]
    );
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
    session.send(Input::Compare(paths)).unwrap();
    let frame = frame(&session);
    assert!(frame.accessible_text.contains("explicit first"));
    assert!(frame.accessible_text.contains("explicit second"));
    session.close();
}

/// A consumer retries inputs a full queue refused, but not inputs for a stopped worker.
#[test]
fn send_tells_a_full_queue_from_a_worker_that_stopped_after_an_internal_error() {
    let root = fixture();
    let paths = paths(root.path());
    let (entered, entry) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel::<()>(1);
    let held = Mutex::new(held);
    let capture: NativeCapture = Arc::new(move |_, _| {
        entered.send(()).unwrap();
        // The worker takes no input while it is held here.
        let _ = held.lock().unwrap().recv_timeout(Duration::from_secs(15));
        panic!("deliberate capture fault: the comparison worker stops");
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = Service::with_native_capture(runtime.handle().clone(), capture);
    let session = open(&service, launch(&paths)).unwrap();
    entry.recv_timeout(Duration::from_secs(15)).unwrap();
    for _ in 0..crate::INPUT_QUEUE {
        session.send(Input::Fit).unwrap();
    }
    assert_eq!(session.send(Input::Fit), Err(Refused::Full));
    release.send(()).unwrap();
    // The worker has dropped its queue by the time it reports the stop.
    wait_for(|last_status| {
        let status = match session.mailbox.take() {
            Some(Output::Status(status)) => status,
            _ => return None,
        };
        let stopped = status.contains("internal error");
        *last_status = Some(status);
        stopped.then_some(())
    });
    assert_eq!(session.send(Input::Fit), Err(Refused::Stopped));
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

#[test]
fn a_failed_image_viewer_discovery_is_explained_and_retried_by_the_next_comparison() {
    let root = fixture();
    let text = paths(root.path());
    let images = [
        root.path().join("left").join("photo.png"),
        root.path().join("right").join("photo.png"),
    ];
    let reads = Arc::new(AtomicUsize::new(0));
    let binary = images.clone();
    let capture: NativeCapture = Arc::new(move |path, _| {
        if binary.iter().any(|image| image == path) {
            memory(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR", &reads)
        } else {
            memory(b"same\nline\n", &reads)
        }
    });
    let discoveries = Arc::new(AtomicUsize::new(0));
    let attempts = discoveries.clone();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = Service::with_native_capture(runtime.handle().clone(), capture)
        .with_font_scan(|_| Some(Arc::from([])))
        .with_image_viewer_discovery(move || {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                Err("viewer packages are being replaced".into())
            } else {
                Ok(Vec::new())
            }
        });
    // Without the viewer list the images were compared as text; the failure says why.
    let first = open(&service, launch(&images)).unwrap();
    let failed = choice(&first);
    assert!(failed.is_error);
    assert!(
        failed
            .message
            .contains("viewer packages are being replaced"),
        "{}",
        failed.message
    );
    assert_eq!(discoveries.load(Ordering::SeqCst), 1);
    assert!(
        service.viewers.kept().is_none(),
        "a failed discovery must not disable image comparison for the session"
    );
    // The next comparison discovers again and keeps the successful result.
    let second = open(&service, launch(&text)).unwrap();
    frame(&second);
    assert_eq!(discoveries.load(Ordering::SeqCst), 2);
    assert!(service.viewers.kept().is_some());
    first.close();
    let third = wait_for(|_| open(&service, launch(&text)).ok());
    frame(&third);
    assert_eq!(discoveries.load(Ordering::SeqCst), 2);
    second.close();
    third.close();
}

fn text_service(runtime: &tokio::runtime::Runtime) -> Service {
    let reads = Arc::new(AtomicUsize::new(0));
    let capture: NativeCapture = Arc::new(move |_, _| memory(b"same\nline\n", &reads));
    Service::with_native_capture(runtime.handle().clone(), capture)
        .with_image_viewer_discovery(|| Ok(Vec::new()))
}

#[test]
fn the_first_frame_is_painted_while_the_font_scan_runs_and_repainted_with_its_catalog() {
    let root = fixture();
    let paths = paths(root.path());
    let (deliver, catalog) = mpsc::channel::<Arc<[String]>>();
    let catalog = Mutex::new(catalog);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service =
        text_service(&runtime).with_font_scan(move |_| catalog.lock().unwrap().recv().ok());
    let session = open_in_family(&service, launch(&paths), "Wide Mono").unwrap();
    let first = frame(&session);
    assert!(
        first.fonts.is_none(),
        "the first frame must not wait for a running font scan"
    );
    deliver.send(Arc::from(["Wide Mono".to_owned()])).unwrap();
    let repainted = frame(&session);
    let fonts = repainted.fonts.expect("the completed catalog is painted");
    assert_eq!(fonts.selected, "Wide Mono");
    session.close();
}

#[test]
fn a_kept_font_catalog_paints_the_first_frame_of_a_later_comparison_in_the_preferred_family() {
    let root = fixture();
    let paths = paths(root.path());
    let scans = Arc::new(AtomicUsize::new(0));
    let scanned = scans.clone();
    let (release, held) = mpsc::channel::<()>();
    let held = Mutex::new(held);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = text_service(&runtime).with_font_scan(move |_| {
        if scanned.fetch_add(1, Ordering::SeqCst) == 0 {
            return Some(Arc::from([
                "Narrow Mono".to_owned(),
                "Wide Mono".to_owned(),
            ]));
        }
        // Another scan would race the first frame; hold it until the test ends, so a
        // comparison that does not use the kept catalog paints without one.
        let _ = held.lock().unwrap().recv();
        None
    });
    let first = open_in_family(&service, launch(&paths), "Wide Mono").unwrap();
    // Whether or not it won the race with the first frame, the catalog is painted.
    let catalog = loop {
        if let Some(fonts) = frame(&first).fonts {
            break fonts;
        }
    };
    assert_eq!(catalog.selected, "Wide Mono");
    let second = open_in_family(&service, launch(&paths), "Wide Mono").unwrap();
    let fonts = frame(&second)
        .fonts
        .expect("the first frame of a later comparison carries the kept catalog");
    assert_eq!(fonts.selected, "Wide Mono");
    assert!(Arc::ptr_eq(&fonts.families, &catalog.families));
    assert_eq!(
        scans.load(Ordering::SeqCst),
        1,
        "the catalog is scanned once"
    );
    first.close();
    second.close();
    drop(release);
}
