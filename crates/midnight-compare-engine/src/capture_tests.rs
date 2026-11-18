use crate::{FileChoice, Frame, Inbox, Input, NativeCapture, Output, Refused, Service, Session};
use midnight_core::{
    DirectoryIndex, Entry, EntryId, EntryKind, FileCompareLaunch, FileCompareSource,
    ViewerAppearance, ViewerGeometry,
};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[path = "identity_lifecycle_tests.rs"]
mod identity_lifecycle;

#[path = "image_identity_tests.rs"]
mod image_identity;

#[derive(Debug)]
struct Memory {
    bytes: &'static [u8],
    reads: Arc<AtomicUsize>,
    gate: Option<Arc<Gate>>,
}

/// Holds the next validation of the sources sharing it, once armed, until released.
#[derive(Debug)]
struct Gate {
    armed: AtomicBool,
    entered: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
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
        if let Some(gate) = &self.gate
            && gate.armed.swap(false, Ordering::AcqRel)
        {
            gate.entered.send(()).unwrap();
            // Longer than any wait for output, so a wait that this hold blocks fails first.
            let _ = gate
                .release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(30));
        }
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
    source(bytes, reads, None)
}

fn source(
    bytes: &'static [u8],
    reads: &Arc<AtomicUsize>,
    gate: Option<&Arc<Gate>>,
) -> Result<CapturedFile, String> {
    CapturedFile::from_source(Arc::new(Memory {
        bytes,
        reads: reads.clone(),
        gate: gate.cloned(),
    }))
    .map_err(|error| error.to_string())
}

fn fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("_build/verification/viewer-refresh-20261117/viewer/compare-identity/captured");
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
    let comparator_snapshot = midnight_core::FileComparatorSnapshot::default();
    FileCompareLaunch {
        sources: paths
            .iter()
            .cloned()
            .map(FileCompareSource::Focused)
            .collect(),
        disabled_plugins: Arc::from([]),
        comparators: comparator_snapshot.preferences.clone(),
        comparator_policy: comparator_snapshot.id,
    }
}

fn unresolved() -> FileCompareLaunch {
    let comparator_snapshot = midnight_core::FileComparatorSnapshot::default();
    FileCompareLaunch {
        sources: Vec::new(),
        disabled_plugins: Arc::from([]),
        comparators: comparator_snapshot.preferences.clone(),
        comparator_policy: comparator_snapshot.id,
    }
}

fn open(service: &Service, launch: FileCompareLaunch) -> Result<Session, crate::OpenError> {
    open_in_family(service, launch, "monospace")
}

fn open_in_family(
    service: &Service,
    launch: FileCompareLaunch,
    family: &str,
) -> Result<Session, crate::OpenError> {
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
        Some(Output::ExternalStarted { .. }) => {
            panic!("Built-in fixture launched an external program")
        }
        Some(Output::Status(status)) => {
            *last_status = Some(status);
            None
        }
        None => None,
    })
}

fn catalog_frame(session: &Session) -> Frame {
    loop {
        let frame = frame(session);
        if frame.fonts.is_some() {
            return frame;
        }
    }
}

fn choice(session: &Session) -> FileChoice {
    wait_for(|last_status| match session.mailbox.take() {
        Some(Output::ChooseFiles(choice)) => Some(choice),
        Some(Output::ExternalStarted { .. }) => {
            panic!("Built-in fixture launched an external program")
        }
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
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
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
fn whole_decoded_text_identity_is_typed_and_does_not_claim_different_bom_bytes_identical() {
    let root = fixture();
    let paths = paths(root.path());
    let expected = paths.clone();
    let reads = Arc::new(AtomicUsize::new(0));
    let capture: NativeCapture = Arc::new(move |path, _| {
        if path == expected[0] {
            memory(b"same text\n", &reads)
        } else if path == expected[1] {
            memory(b"\xef\xbb\xbfsame text\n", &reads)
        } else {
            Err("Unexpected provider identity".into())
        }
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture);
    let session = open(&service, launch(&paths)).unwrap();
    let result = wait_for(|last_status| match session.mailbox.take() {
        Some(Output::Frame { frame, .. }) if frame.identity != crate::IdentityVerdict::Pending => {
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
    });
    assert_eq!(result.identity, crate::IdentityVerdict::IdenticalText);
    assert!(result.comparison_generation > 0);
    assert!(result.status.contains("UTF-8 without BOM"));
    assert!(result.status.contains("UTF-8 with BOM"));
    session.close();
}

#[test]
fn complete_utf16_text_identity_reports_each_actual_byte_order_and_bom() {
    let root = fixture();
    let paths = paths(root.path());
    let expected = paths.clone();
    let reads = Arc::new(AtomicUsize::new(0));
    let capture: NativeCapture = Arc::new(move |path, _| {
        if path == expected[0] {
            memory(&[0xff, 0xfe, 0xe9, 0, 0x0a, 0], &reads)
        } else {
            memory(&[0xfe, 0xff, 0, 0xe9, 0, 0x0a], &reads)
        }
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture);
    let session = open(&service, launch(&paths)).unwrap();
    let result = wait_for(|_| match session.mailbox.take() {
        Some(Output::Frame { frame, .. }) if frame.identity != crate::IdentityVerdict::Pending => {
            Some(frame)
        }
        Some(Output::ChooseFiles(choice)) => panic!("Unexpected file choice: {choice:?}"),
        Some(Output::ExternalStarted { .. }) => {
            panic!("Built-in fixture launched an external program")
        }
        _ => None,
    });
    assert_eq!(result.identity, crate::IdentityVerdict::IdenticalText);
    assert!(result.accessible_text.contains('é'));
    assert!(
        result
            .status
            .contains("File A UTF-16 little-endian with BOM")
    );
    assert!(result.status.contains("File B UTF-16 big-endian with BOM"));
    session.close();
}

#[test]
fn differences_outside_the_visible_text_page_still_produce_typed_different() {
    let root = fixture();
    let paths = paths(root.path());
    for path in &paths {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    std::fs::write(&paths[0], format!("{}left tail\n", "same\n".repeat(500))).unwrap();
    std::fs::write(&paths[1], format!("{}right tail\n", "same\n".repeat(500))).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::new(runtime.handle().clone(), spawner);
    let session = open(&service, launch(&paths)).unwrap();
    let result = wait_for(|_| match session.mailbox.take() {
        Some(Output::Frame { frame, .. }) if frame.identity != crate::IdentityVerdict::Pending => {
            Some(frame)
        }
        Some(Output::ChooseFiles(choice)) => panic!("Unexpected file choice: {choice:?}"),
        Some(Output::ExternalStarted { .. }) => {
            panic!("Built-in fixture launched an external program")
        }
        _ => None,
    });
    assert_eq!(result.identity, crate::IdentityVerdict::Different);
    assert!(!result.accessible_text.contains("left tail"));
    assert!(!result.accessible_text.contains("right tail"));
    session.close();
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
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture);
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
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture);
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
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture);
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
        let (_owner, spawner) = crate::resident_test_support::workers();
        let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture);
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
        let (_owner, spawner) = crate::resident_test_support::workers();
        let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture);
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
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
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

fn text_service(
    runtime: &tokio::runtime::Runtime,
    spawner: midnight_work_budget::resident::Spawner,
) -> Service {
    let reads = Arc::new(AtomicUsize::new(0));
    let capture: NativeCapture = Arc::new(move |_, _| memory(b"same\nline\n", &reads));
    Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_image_viewer_discovery(|| Ok(Vec::new()))
}

#[test]
fn the_first_frame_is_painted_while_the_font_scan_runs_and_repainted_with_its_catalog() {
    let root = fixture();
    let paths = paths(root.path());
    let (deliver, catalog) = mpsc::channel::<Arc<[String]>>();
    let catalog = Mutex::new(catalog);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = text_service(&runtime, spawner)
        .with_font_scan(move |_| catalog.lock().unwrap().recv().ok());
    let session = open_in_family(&service, launch(&paths), "Wide Mono").unwrap();
    let first = frame(&session);
    assert!(
        first.fonts.is_none(),
        "the first frame must not wait for a running font scan"
    );
    deliver.send(Arc::from(["Wide Mono".to_owned()])).unwrap();
    let repainted = catalog_frame(&session);
    let fonts = repainted.fonts.expect("the completed catalog is painted");
    assert_eq!(fonts.selected, "Wide Mono");
    session.close();
}

/// A scan that runs until the returned sender sends or is dropped, counting its runs.
fn held_scan(service: Service, scans: &Arc<AtomicUsize>) -> (Service, mpsc::Sender<Arc<[String]>>) {
    let (deliver, catalog) = mpsc::channel::<Arc<[String]>>();
    let catalog = Mutex::new(catalog);
    let scanned = scans.clone();
    let service = service.with_font_scan(move |_| {
        scanned.fetch_add(1, Ordering::SeqCst);
        catalog.lock().unwrap().recv().ok()
    });
    (service, deliver)
}

#[test]
fn the_first_comparison_paints_in_the_preferred_family_while_the_scan_runs() {
    let root = fixture();
    let paths = paths(root.path());
    let checks = Arc::new(Mutex::new(Vec::new()));
    let checked = checks.clone();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let (service, deliver) = held_scan(text_service(&runtime, spawner), &Arc::default());
    let service = service.with_font_check(move |candidates, _| {
        checked.lock().unwrap().push(
            candidates
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
        );
        candidates
            .iter()
            .find(|name| name.eq_ignore_ascii_case("cascadia mono"))
            .map(|_| "Cascadia Mono".to_owned())
    });
    let session = open_in_family(&service, launch(&paths), "cascadia mono").unwrap();
    let first = frame(&session);
    assert!(first.fonts.is_none(), "the chooser waits for the catalog");
    assert_eq!(first.text_family.as_deref(), Some("Cascadia Mono"));
    assert_eq!(
        *checks.lock().unwrap(),
        [["cascadia mono", "Consolas"].map(str::to_owned)],
        "one check of at most two families precedes the first frame"
    );
    deliver
        .send(Arc::from([
            "Cascadia Mono".to_owned(),
            "Consolas".to_owned(),
        ]))
        .unwrap();
    let repainted = catalog_frame(&session);
    let fonts = repainted
        .fonts
        .expect("the catalog publishes the chooser list");
    assert_eq!(fonts.selected, "Cascadia Mono");
    assert_eq!(repainted.text_family.as_deref(), Some("Cascadia Mono"));
    assert_eq!(checks.lock().unwrap().len(), 1);
    session.close();
}

#[test]
fn a_missing_preferred_family_falls_back_to_consolas_then_monospace_before_the_catalog() {
    let root = fixture();
    let paths = paths(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    for (installed, painted) in [(Some("Consolas"), "Consolas"), (None, "monospace")] {
        let (service, release) = held_scan(text_service(&runtime, spawner), &Arc::default());
        let service = service.with_font_check(move |candidates, _| {
            installed
                .filter(|family| candidates.contains(family))
                .map(str::to_owned)
        });
        let session = open_in_family(&service, launch(&paths), "Missing Mono").unwrap();
        let first = frame(&session);
        assert!(first.fonts.is_none());
        assert_eq!(first.text_family.as_deref(), Some(painted));
        session.close();
        drop(release);
    }
}

/// The worker blocks while the catalog loads; only the scan's reply wakes it. The window
/// below observes the absence of turns; nothing waits on it.
#[test]
fn no_worker_turn_while_the_catalog_is_pending() {
    let root = fixture();
    let paths = paths(root.path());
    let turns = Arc::new(AtomicUsize::new(0));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let (service, deliver) = held_scan(text_service(&runtime, spawner), &Arc::default());
    let service = service
        .with_font_check(|_, _| None)
        .with_turn_counter(turns.clone());
    let session = open(&service, launch(&paths)).unwrap();
    // Leave the frame unread so whole-input verification is also waiting.
    let has_catalog = || {
        let state = session.mailbox.state.lock().unwrap();
        match &state.output {
            Some(Output::Frame { frame, .. }) => Some(frame.fonts.is_some()),
            _ => None,
        }
    };
    assert!(!wait_for(|_| has_catalog()));
    assert_eq!(
        turns.load(Ordering::SeqCst),
        1,
        "one turn painted the first frame"
    );
    // A polling worker would turn about every 20 ms in this window.
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(
        turns.load(Ordering::SeqCst),
        1,
        "no worker turn while the catalog loads"
    );
    deliver.send(Arc::from(["Wide Mono".to_owned()])).unwrap();
    wait_for(|_| (has_catalog() == Some(true)).then_some(()));
    assert_eq!(
        turns.load(Ordering::SeqCst),
        2,
        "the reply alone woke the worker"
    );
    session.close();
}

#[test]
fn a_comparison_after_a_failed_one_gets_the_catalog_of_the_same_scan() {
    let root = fixture();
    let text = paths(root.path());
    let binary = [
        root.path().join("left").join("binary.txt"),
        root.path().join("right").join("binary.txt"),
    ];
    let reads = Arc::new(AtomicUsize::new(0));
    let failing = binary.clone();
    let capture: NativeCapture = Arc::new(move |path, _| {
        if failing.iter().any(|side| side == path) {
            memory(b"control \x01 byte\n", &reads)
        } else {
            memory(b"same\nline\n", &reads)
        }
    });
    let scans = Arc::new(AtomicUsize::new(0));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let (service, deliver) = held_scan(
        Service::with_native_capture(runtime.handle().clone(), spawner, capture)
            .with_image_viewer_discovery(|| Ok(Vec::new())),
        &scans,
    );
    let service = service.with_font_check(|_, _| None);
    let session = open_in_family(&service, launch(&binary), "Wide Mono").unwrap();
    // The failed comparison requested the catalog before it failed.
    assert!(choice(&session).is_error);
    session.send(Input::Compare(text)).unwrap();
    assert!(frame(&session).fonts.is_none());
    deliver.send(Arc::from(["Wide Mono".to_owned()])).unwrap();
    let fonts = frame(&session)
        .fonts
        .expect("the waiting comparison gets the catalog");
    assert_eq!(fonts.selected, "Wide Mono");
    assert_eq!(
        scans.load(Ordering::SeqCst),
        1,
        "one scan serves both requests"
    );
    session.close();
}

#[test]
fn a_scan_without_a_thread_answers_at_once_even_while_the_input_queue_is_full() {
    let root = fixture();
    let paths = paths(root.path());
    let (entered, entry) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel::<()>(1);
    let held = Mutex::new(held);
    let reads = Arc::new(AtomicUsize::new(0));
    let first = paths[0].clone();
    let capture: NativeCapture = Arc::new(move |path, _| {
        if path == first {
            entered.send(()).unwrap();
            // The worker takes no input while it is held here.
            let _ = held.lock().unwrap().recv_timeout(Duration::from_secs(15));
        }
        memory(b"same\nline\n", &reads)
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_image_viewer_discovery(|| Ok(Vec::new()))
        .without_font_thread();
    let session = open(&service, launch(&paths)).unwrap();
    entry.recv_timeout(Duration::from_secs(15)).unwrap();
    for _ in 0..crate::INPUT_QUEUE {
        session.send(Input::Fit).unwrap();
    }
    release.send(()).unwrap();
    let fonts = frame(&session)
        .fonts
        .expect("an empty catalog is answered at once");
    assert!(fonts.families.is_empty());
    assert_eq!(fonts.selected, "monospace");
    session.close();
}

#[test]
fn a_panicking_scan_answers_its_waiters_and_the_next_comparison_scans_again() {
    let root = fixture();
    let paths = paths(root.path());
    let turns = Arc::new(AtomicUsize::new(0));
    let scans = Arc::new(AtomicUsize::new(0));
    let scanned = scans.clone();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = text_service(&runtime, spawner)
        .with_font_scan(move |_| {
            if scanned.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("deliberate font scan fault");
            }
            Some(Arc::from(["Wide Mono".to_owned()]))
        })
        .with_font_check(|_, _| None)
        .with_turn_counter(turns.clone());
    let first = open_in_family(&service, launch(&paths), "Wide Mono").unwrap();
    // The panicking scan's reply (`None`) wakes the worker before any frame is taken.
    wait_for(|_| (turns.load(Ordering::SeqCst) >= 2).then_some(()));
    assert!(frame(&first).fonts.is_none());
    let second = open_in_family(&service, launch(&paths), "Wide Mono").unwrap();
    let fonts = loop {
        if let Some(fonts) = frame(&second).fonts {
            break fonts;
        }
    };
    assert_eq!(fonts.selected, "Wide Mono");
    assert_eq!(scans.load(Ordering::SeqCst), 2);
    first.close();
    second.close();
}

/// A comparison that fails after its font request leaves that request's scan running. Here
/// that scan panics while the next comparison opens its files; its reply (no catalog) must
/// not end the wait of the next comparison, which scans again and publishes that catalog.
#[test]
fn a_failed_scan_of_an_earlier_comparison_does_not_answer_the_next_one() {
    let root = fixture();
    let text = paths(root.path());
    let binary = [
        root.path().join("left").join("binary.txt"),
        root.path().join("right").join("binary.txt"),
    ];
    let (entered, entry) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel::<()>(1);
    let held = Mutex::new(held);
    let reads = Arc::new(AtomicUsize::new(0));
    let failing = binary.clone();
    let opened = text[0].clone();
    let capture: NativeCapture = Arc::new(move |path, _| {
        if failing.iter().any(|side| side == path) {
            return memory(b"control \x01 byte\n", &reads);
        }
        if path == opened {
            entered.send(()).unwrap();
            let _ = held.lock().unwrap().recv_timeout(Duration::from_secs(15));
        }
        memory(b"same\nline\n", &reads)
    });
    let scans = Arc::new(AtomicUsize::new(0));
    let scanned = scans.clone();
    let (fail, failure) = mpsc::sync_channel::<()>(1);
    let (finish, finished) = mpsc::sync_channel::<()>(1);
    let (failure, finished) = (Mutex::new(failure), Mutex::new(finished));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_image_viewer_discovery(|| Ok(Vec::new()))
        .with_font_scan(move |_| {
            if scanned.fetch_add(1, Ordering::SeqCst) == 0 {
                let _ = failure
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(15));
                panic!("deliberate font scan fault");
            }
            let _ = finished
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15));
            Some(Arc::from(["Wide Mono".to_owned()]))
        })
        .with_font_check(|_, _| None);
    let session = open_in_family(&service, launch(&binary), "Wide Mono").unwrap();
    // The first comparison requested the catalog, then failed on the binary content.
    assert!(choice(&session).is_error);
    session.send(Input::Compare(text)).unwrap();
    entry.recv_timeout(Duration::from_secs(15)).unwrap();
    fail.send(()).unwrap();
    // The failed scan's reply to the first request waits while the files are opened.
    wait_for(|_| (session.inbox.answered() == Some(1)).then_some(()));
    release.send(()).unwrap();
    assert!(
        frame(&session).fonts.is_none(),
        "the first scan's reply is not the second comparison's"
    );
    finish.send(()).unwrap();
    let fonts = loop {
        if let Some(fonts) = frame(&session).fonts {
            break fonts;
        }
    };
    assert_eq!(fonts.selected, "Wide Mono");
    assert_eq!(
        scans.load(Ordering::SeqCst),
        2,
        "the second comparison scans again"
    );
    session.close();
}

/// One scan answers both comparison windows. The worker of the first is busy and its
/// queue full; that must neither delay the second window's reply nor lose its own: the
/// full queue refuses its wake, and it reads the reply once it has taken its inputs.
#[test]
fn a_window_with_a_full_queue_neither_delays_nor_loses_a_catalog_reply() {
    let root = fixture();
    let busy = paths(root.path());
    let idle = [
        root.path().join("left").join("idle.txt"),
        root.path().join("right").join("idle.txt"),
    ];
    let (entered, entry) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel(1);
    let gate = Arc::new(Gate {
        armed: AtomicBool::new(false),
        entered,
        release: Mutex::new(held),
    });
    let reads = Arc::new(AtomicUsize::new(0));
    let gated = gate.clone();
    let capture: NativeCapture = Arc::new(move |path, _| {
        let gate = busy.iter().any(|side| side == path).then_some(&gated);
        source(b"same\nline\n", &reads, gate)
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let (service, deliver) = held_scan(
        Service::with_native_capture(runtime.handle().clone(), spawner, capture)
            .with_image_viewer_discovery(|| Ok(Vec::new())),
        &Arc::default(),
    );
    let service = service.with_font_check(|_, _| None);
    let first = open_in_family(&service, launch(&paths(root.path())), "Wide Mono").unwrap();
    assert!(frame(&first).fonts.is_none());
    let second = open_in_family(&service, launch(&idle), "Wide Mono").unwrap();
    assert!(frame(&second).fonts.is_none());
    // Finish the first worker's identity checks before arming the validation hold.
    wait_for(|_| match first.mailbox.take() {
        Some(Output::Frame { frame, .. }) if frame.identity != crate::IdentityVerdict::Pending => {
            Some(())
        }
        _ => None,
    });
    // The first worker repaints and is held in revalidating its source.
    gate.armed.store(true, Ordering::Release);
    first
        .send(Input::Resize(
            ViewerGeometry {
                width: 600,
                height: 200,
                scale: 1.0,
            },
            1,
        ))
        .unwrap();
    entry.recv_timeout(Duration::from_secs(15)).unwrap();
    for _ in 0..crate::INPUT_QUEUE {
        first.send(Input::Fit).unwrap();
    }
    assert_eq!(first.send(Input::Fit), Err(Refused::Full));
    deliver.send(Arc::from(["Wide Mono".to_owned()])).unwrap();
    let fonts = loop {
        if let Some(fonts) = frame(&second).fonts {
            break fonts;
        }
    };
    assert_eq!(fonts.selected, "Wide Mono");
    release.send(()).unwrap();
    let fonts = loop {
        if let Some(fonts) = frame(&first).fonts {
            break fonts;
        }
    };
    assert_eq!(fonts.selected, "Wide Mono");
    first.close();
    second.close();
}

/// Neither `Session::close` nor a font catalog reply needs a free queue slot: the worker
/// reads both from the inbox before it blocks. A late reply to an earlier request neither
/// replaces the newer reply nor answers the newer request.
#[test]
fn the_inbox_keeps_a_close_and_the_current_reply_that_a_full_queue_refused() {
    let (sender, receiver) = mpsc::sync_channel(crate::INPUT_QUEUE);
    let inbox = Inbox::new(sender);
    for _ in 0..crate::INPUT_QUEUE {
        inbox
            .send(crate::Command {
                input: Input::Fit,
                generation: 0,
            })
            .unwrap();
    }
    assert_eq!(
        inbox.send(crate::Command {
            input: Input::Fit,
            generation: 0
        }),
        Err(Refused::Full)
    );
    let earlier = inbox.font_request();
    let current = inbox.font_request();
    let catalog: Arc<[String]> = Arc::from(["Wide Mono".to_owned()]);
    inbox.answer(current, Some(catalog.clone()));
    inbox.answer(earlier, None);
    assert_eq!(inbox.answered(), Some(current));
    assert_eq!(inbox.check(Some(current)), Ok(Some(Some(catalog))));
    inbox.answer(earlier, None);
    assert_eq!(
        inbox.check(Some(current)),
        Ok(None),
        "a reply to an earlier request is dropped"
    );
    assert_eq!(inbox.answered(), None);
    inbox.close();
    assert!(
        inbox.check(None).is_err(),
        "the refused Close still ends the worker"
    );
    assert_eq!(
        receiver.try_iter().count(),
        crate::INPUT_QUEUE,
        "the full queue refused every wake and Close"
    );
}

/// Opt-in timing of a process's first text comparison with the installed font scan and
/// check: its first frame (and that frame's family) and the frame that publishes the
/// catalog. Run optimized, in a fresh process each time:
/// `cargo test --release -p midnight-compare-engine --lib first_comparison_font_timing -- --ignored --nocapture`
#[test]
#[ignore = "timing harness; run optimized with --ignored --nocapture"]
fn first_comparison_font_timing() {
    let root = fixture();
    let paths = paths(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = text_service(&runtime, spawner);
    let (woken, wakes) = mpsc::sync_channel(1);
    let started = Instant::now();
    let session = service
        .open_with_wake(
            launch(&paths),
            ViewerAppearance {
                family: "Consolas".into(),
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
            Arc::new(move || {
                let _ = woken.try_send(());
                true
            }),
        )
        .unwrap();
    let mut first = None;
    let catalog = loop {
        wakes
            .recv_timeout(Duration::from_secs(60))
            .expect("an output within a minute");
        if let Some(Output::Frame { frame, .. }) = session.mailbox.take() {
            let at = started.elapsed();
            first.get_or_insert((at, frame.text_family.clone()));
            if frame.fonts.is_some() {
                break at;
            }
        }
    };
    let (first, family) = first.unwrap();
    println!(
        "compare_font_timing first_frame_ms={:.1} first_family={family:?} catalog_frame_ms={:.1}",
        first.as_secs_f64() * 1000.0,
        catalog.as_secs_f64() * 1000.0
    );
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
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = text_service(&runtime, spawner).with_font_scan(move |_| {
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
