#[path = "../examples/support/resident.rs"]
mod resident;

use super::*;
use slint::platform::{Platform, PlatformError, WindowAdapter};
use std::{
    io,
    sync::{Arc, mpsc},
    thread::ThreadId,
};

#[derive(Debug)]
struct Source(mpsc::Sender<ThreadId>);
impl Drop for Source {
    fn drop(&mut self) {
        let _ = self.0.send(std::thread::current().id());
    }
}
impl GeneratedTextSource for Source {
    fn length(&self) -> u64 {
        0
    }
    fn read_at(&self, _: u64, _: &mut [u8]) -> io::Result<usize> {
        panic!("rejected source read")
    }
}
fn failed_open(manager: &ViewerWindows, generated: Option<Arc<dyn GeneratedTextSource>>) -> String {
    let path = std::env::temp_dir().join("diagnostic.txt");
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    let result = manager.open(
        ViewerLaunch {
            find: None,
            associations: Default::default(),
            tree: None,
            archive: None,
            generated,
            focused: path,
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
            preferences: Arc::default(),
            basic: true,
            disabled_plugins: Arc::from([]),
        },
        ViewerAppearance {
            family: "monospace".into(),
            font_size: 14.0,
            foreground: [255; 4],
            background: [0, 0, 0, 255],
            selection: [55; 4],
            selection_text: [0; 4],
            accent: [150; 4],
        },
        slint::LogicalSize::new(320.0, 200.0),
        Rc::new(|_, _| None),
        Rc::new(|_| {}),
    );
    result.err().expect("window unexpectedly opened")
}

fn reject(manager: &ViewerWindows, expected: &str) {
    let (sender, dropped) = mpsc::channel();
    let error = failed_open(manager, Some(Arc::new(Source(sender))));
    assert!(error.contains(expected), "{error}");
    assert_ne!(
        dropped.recv_timeout(Duration::from_secs(5)).unwrap(),
        std::thread::current().id(),
        "diagnostic source dropped on UI thread"
    );
}

#[test]
fn manager_window_limit_retires_source_on_backend() {
    let (_runtime, manager, open, _service) = super::opening_tests::opening(Box::new(|_| {}));
    manager
        .windows
        .borrow_mut()
        .extend([open.clone(), open.clone(), open]);
    reject(&manager, "maximum four windows");
    manager.close_all();
}

struct BrokenPlatform;
impl Platform for BrokenPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Err(PlatformError::Other("Window creation failed".into()))
    }
}
#[test]
fn native_window_failure_retires_source_on_backend() {
    slint::platform::set_platform(Box::new(BrokenPlatform)).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let runtime = resident::ResidentRuntime::new(runtime).unwrap();
    let service = Service::new(runtime.handle().clone(), runtime.spawner());
    runtime.track_spare(&service);
    let manager = ViewerWindows::new(service);
    // More than the engine's four session permits: every failed window releases
    // its source on a worker and returns its admission slot.
    for _ in 0..6 {
        reject(&manager, "Window creation failed");
    }
}

struct WaitForCapture(mpsc::Receiver<ThreadId>);
impl Platform for WaitForCapture {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let error = match self.0.recv_timeout(Duration::from_secs(10)) {
            Ok(id) if id != std::thread::current().id() => "native failure after capture began",
            _ => "native construction waited without a worker capture",
        };
        Err(PlatformError::Other(error.into()))
    }
}

#[test]
fn capture_starts_before_native_window_creation_finishes() {
    let (started, capture) = mpsc::channel();
    slint::platform::set_platform(Box::new(WaitForCapture(capture))).unwrap();
    let runtime = resident::ResidentRuntime::new(tokio::runtime::Runtime::new().unwrap()).unwrap();
    let service = Service::with_native_capture(
        runtime.handle().clone(),
        runtime.spawner(),
        Arc::new(move |_, _| {
            let _ = started.send(std::thread::current().id());
            Err("fixture capture ended".into())
        }),
    );
    runtime.track_spare(&service);
    let error = failed_open(&ViewerWindows::new(service), None);
    assert!(
        error.contains("native failure after capture began"),
        "{error}"
    );
}
