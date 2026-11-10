use super::*;
use slint::platform::{Platform, PlatformError, WindowAdapter};
use std::{
    io,
    path::PathBuf,
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
fn reject(manager: &ViewerWindows) {
    let (sender, dropped) = mpsc::channel();
    let path = PathBuf::from("diagnostic.txt");
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    let result = manager.open(
        ViewerLaunch {
            associations: Default::default(),
            tree: None,
            archive: None,
            generated: Some(Arc::new(Source(sender))),
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
            accent: [150; 4],
        },
        slint::LogicalSize::new(320.0, 200.0),
        Rc::new(|_, _| None),
        Rc::new(|_| {}),
    );
    assert!(result.is_err());
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
    reject(&manager);
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
    let manager = ViewerWindows::new(Service::new(runtime.handle().clone()));
    reject(&manager);
}
