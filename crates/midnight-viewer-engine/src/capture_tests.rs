//! The injected filesystem service owns native capture; errors never reopen paths.
use crate::{NativeCapture, capture};
use midnight_core::{DirectoryIndex, Entry, EntryId, EntryKind, ViewerLaunch};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Debug)]
struct Memory;
impl ContentSource for Memory {
    fn length(&self) -> u64 {
        8
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
        let start = offset.min(8) as usize;
        let end = start.saturating_add(length as usize).min(8);
        Ok(b"provider"[start..end].to_vec())
    }
}

fn launch(root: &Path) -> ViewerLaunch {
    let entries: Arc<[Entry]> = ["first.txt", "second.txt", "hidden.txt"]
        .into_iter()
        .enumerate()
        .map(|(id, name)| Entry::new(EntryId(id as u64), root.join(name), EntryKind::File))
        .collect();
    let mut view = DirectoryIndex::from_entries(&entries);
    view.by_id.remove(&EntryId(2));
    view.by_id.insert(EntryId(0), 1);
    view.by_id.insert(EntryId(1), 0);
    ViewerLaunch {
        archive: None,
        focused: entries[0].path.clone(),
        entries,
        view: Arc::new(view),
        basic: true,
        disabled_plugins: Arc::from([]),
    }
}

#[tokio::test]
async fn injected_capture_owns_focused_and_next_visible_content_without_native_files() {
    let root = tempfile::tempdir().unwrap();
    let launch = launch(root.path());
    let calls = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let record = calls.clone();
    let source: NativeCapture = Arc::new(move |path, closed| {
        assert!(!closed.load(Ordering::Acquire));
        record.lock().unwrap().push(path.into());
        CapturedFile::from_source(Arc::new(Memory)).map_err(|error| error.to_string())
    });
    let closed = Arc::new(AtomicBool::new(false));
    let order = capture::prepare_with(&launch, &closed, source).unwrap();
    assert_eq!(order.current, 1);
    assert_eq!(order.files.len(), 2);
    let focused = order.files[order.current]
        .1
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(focused.read_at(0, 8).unwrap(), b"provider");
    let (next, file) = order.ready.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(next, 0);
    assert_eq!(file.unwrap().read_at(0, 8).unwrap(), b"provider");
    assert_eq!(
        *calls.lock().unwrap(),
        [
            root.path().join("first.txt"),
            root.path().join("second.txt")
        ]
    );
}

#[tokio::test]
async fn unavailable_injected_provider_never_falls_back_to_a_readable_native_file() {
    let root = tempfile::tempdir().unwrap();
    let launch = launch(root.path());
    std::fs::write(&launch.focused, b"native bytes must not be opened").unwrap();
    let source: NativeCapture = Arc::new(|_, _| Err("Provider removed".into()));
    let closed = Arc::new(AtomicBool::new(false));
    let order = capture::prepare_with(&launch, &closed, source).unwrap();
    assert_eq!(
        order.files[order.current]
            .1
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err(),
        "Provider removed"
    );
    assert_eq!(
        order
            .ready
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .1
            .unwrap_err(),
        "Provider removed"
    );
}

#[tokio::test]
async fn close_during_focused_capture_discards_result_and_prevents_prefetch() {
    let root = tempfile::tempdir().unwrap();
    let launch = launch(root.path());
    let calls = Arc::new(Mutex::new(0));
    let count = calls.clone();
    let source: NativeCapture = Arc::new(move |_, closed| {
        *count.lock().unwrap() += 1;
        closed.store(true, Ordering::Release);
        CapturedFile::from_source(Arc::new(Memory)).map_err(|error| error.to_string())
    });
    let closed = Arc::new(AtomicBool::new(false));
    assert!(capture::prepare_with(&launch, &closed, source).is_err());
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[tokio::test]
async fn a_virtual_entry_cannot_enter_the_native_capture_route() {
    let root = tempfile::tempdir().unwrap();
    let mut launch = launch(root.path());
    let mut entries = launch.entries.to_vec();
    entries[0].virtual_node = Some(midnight_core::ArchiveNode {
        id: midnight_core::VirtualEntryId { mount: 1, entry: 2 },
        parent: 1,
        raw_name: Arc::from(b"first.txt".as_slice()),
        display_name: "first.txt".into(),
        kind: EntryKind::File,
        size: Some(8),
        readable: true,
    });
    launch.entries = entries.into();
    let source: NativeCapture = Arc::new(|_, _| panic!("native capture must not run"));
    assert!(
        capture::prepare_with(&launch, &Arc::new(AtomicBool::new(false)), source)
            .err()
            .unwrap()
            .contains("non-native")
    );
}

#[test]
fn viewer_service_dispatches_injected_capture_on_a_backend_thread() {
    let root = tempfile::tempdir().unwrap();
    let launch = launch(root.path());
    let focused = launch.focused.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let source: NativeCapture = Arc::new(move |path, _| {
        sender
            .send((path.to_path_buf(), std::thread::current().id()))
            .unwrap();
        Err("Injected provider unavailable".into())
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let service = crate::Service::with_native_capture(runtime.handle().clone(), source);
    *service.runtime.lock().unwrap() = Some(Arc::new(
        midnight_plugin_host::Runtime::discover(root.path()).unwrap(),
    ));
    *service.images.lock().unwrap() = Some(Arc::from([]));
    let session = service
        .open(
            launch,
            midnight_core::ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [255; 4],
                background: [0, 0, 0, 255],
                selection: [80, 100, 180, 255],
                accent: [150, 180, 240, 255],
            },
            midnight_core::ViewerGeometry {
                width: 320,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap();
    let (path, thread) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(path, focused);
    assert_ne!(thread, std::thread::current().id());
    session.close();
    runtime.shutdown_timeout(Duration::from_secs(2));
}
