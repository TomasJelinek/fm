use super::*;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::sync::Arc;

struct TestPlatform;
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}

pub(super) fn opening(
    ready: OpenReady,
) -> (
    tokio::runtime::Runtime,
    ViewerWindows,
    Rc<OpenWindow>,
    Service,
) {
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = Service::with_native_capture(
        runtime.handle().clone(),
        Arc::new(|_, _| Err("captured source unavailable".into())),
    );
    let manager = ViewerWindows::new(service.clone());
    let path = std::env::temp_dir().join("unavailable-generated-viewer.txt");
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    manager
        .open_with_ready(
            ViewerLaunch {
                find: None,
                associations: Default::default(),
                tree: None,
                generated: None,
                focused: path,
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
                preferences: Arc::default(),
                archive: None,
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
            ready,
        )
        .unwrap();
    let open = manager.windows.borrow()[0].clone();
    (runtime, manager, open, service)
}

#[test]
fn closing_before_initial_content_notifies_cancellation_once() {
    let results = Rc::new(RefCell::new(Vec::new()));
    let received = results.clone();
    let (_runtime, manager, open, service) =
        opening(Box::new(move |result| received.borrow_mut().push(result)));
    close(&open);
    close(&open);
    present_output(&open, &service, &|_| {});
    assert_eq!(results.borrow().len(), 1);
    assert!(results.borrow()[0].as_ref().unwrap_err().contains("closed"));
    manager.close_all();
}

#[test]
fn initial_backend_failure_is_delivered_once_and_closing_cannot_replace_it() {
    let results = Rc::new(RefCell::new(Vec::new()));
    let received = results.clone();
    let (_runtime, manager, open, service) =
        opening(Box::new(move |result| received.borrow_mut().push(result)));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while results.borrow().is_empty() && std::time::Instant::now() < deadline {
        present_output(&open, &service, &|_| {});
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        results.borrow().len(),
        1,
        "actual backend failure was not delivered"
    );
    assert!(results.borrow()[0].is_err());
    close(&open);
    assert_eq!(results.borrow().len(), 1);
    manager.close_all();
}

#[test]
fn a_control_batch_cannot_report_success_before_the_first_prepared_frame() {
    let results = Rc::new(RefCell::new(Vec::new()));
    let received = results.clone();
    let (_runtime, manager, open, _service) =
        opening(Box::new(move |result| received.borrow_mut().push(result)));
    open.acknowledge_opening(Some(Ok(())));
    open.acknowledge_opening(None);
    assert!(results.borrow().is_empty());
    open.frame.set(1);
    open.acknowledge_opening(None);
    assert_eq!(&*results.borrow(), &[Ok(())]);
    close(&open);
    assert_eq!(results.borrow().len(), 1);
    manager.close_all();
}
