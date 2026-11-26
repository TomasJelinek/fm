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

type Opening = (
    image_presentation_tests::TestRuntime,
    ViewerWindows,
    Rc<OpenWindow>,
    Service,
);

pub(super) fn opening(ready: OpenReady) -> Opening {
    opening_with(ready, |_| {})
}

fn opening_with(ready: OpenReady, configure: impl FnOnce(&ViewerWindows)) -> Opening {
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    let runtime = image_presentation_tests::TestRuntime::new();
    let (release, capture) = std::sync::mpsc::channel::<()>();
    let capture = std::sync::Mutex::new(capture);
    let service = Service::with_native_capture(
        runtime.handle().clone(),
        runtime.spawner(),
        Arc::new(move |_, _| {
            // Release only after frontend setup. Dropping release on any setup
            // exit also unblocks this actual capture before Runtime teardown.
            let _ = capture.lock().unwrap().recv();
            Err("captured source unavailable".into())
        }),
    );
    let manager = ViewerWindows::new(service.clone());
    configure(&manager);
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
                selection_text: [0; 4],
                accent: [150; 4],
            },
            slint::LogicalSize::new(320.0, 200.0),
            Rc::new(|_, _| None),
            Rc::new(|_| {}),
            ready,
        )
        .unwrap();
    let open = manager.windows.borrow()[0].clone();
    drop(release);
    (runtime, manager, open, service)
}

#[test]
fn host_icon_is_applied_at_open_and_updates_the_live_window() {
    let initial = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(2, 2));
    let changed = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(3, 3));
    let (_runtime, manager, open, _service) = opening_with(Box::new(|_| {}), |manager| {
        manager.set_app_icon(initial.clone())
    });
    let displayed_at_open = open.window.get_app_icon();
    let initial_palette = open.session.palette_revision();
    manager.set_app_icon(changed.clone());
    let displayed_after_change = open.window.get_app_icon();
    let final_palette = open.session.palette_revision();
    manager.close_all();
    assert!(open.closed.get());
    assert_eq!(displayed_at_open, initial);
    assert_eq!(displayed_after_change, changed);
    assert_eq!(final_palette, initial_palette);
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
fn cancellation_callback_may_reenter_the_manager_while_closing_all_windows() {
    let manager_slot: Rc<RefCell<std::rc::Weak<ViewerWindows>>> = Rc::default();
    let reentered = Rc::new(Cell::new(false));
    let (slot, flag) = (manager_slot.clone(), reentered.clone());
    let (_runtime, manager, open, _service) = opening(Box::new(move |result| {
        assert!(result.is_err());
        if let Some(manager) = slot.borrow().upgrade() {
            // OpenReady runs synchronously inside close_all and may use the manager.
            manager.restore_preferences(Arc::default());
            manager.close_all();
            flag.set(true);
        }
    }));
    let manager = Rc::new(manager);
    *manager_slot.borrow_mut() = Rc::downgrade(&manager);
    manager.close_all();
    assert!(reentered.get());
    assert!(open.closed.get());
    assert!(manager.windows.borrow().is_empty());
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

#[test]
fn an_opening_takes_the_prepared_spare_component_and_leaves_none_behind() {
    let prepared = Rc::new(Cell::new(std::ptr::null::<slint::Window>()));
    let recorded = prepared.clone();
    let (_runtime, manager, open, _service) = opening_with(Box::new(|_| {}), move |manager| {
        let spare = ViewerWindow::new().unwrap();
        recorded.set(spare.window() as *const slint::Window);
        *manager.spare.window.borrow_mut() = Some(spare);
    });
    assert!(
        std::ptr::eq(open.window.window(), prepared.get()),
        "the opening shows the component built ahead of it"
    );
    assert!(
        manager.spare.window.borrow().is_none(),
        "a spare is used once; the next one is built later"
    );
    manager.close_all();
}

#[test]
fn a_released_spare_is_dropped_and_not_refilled_by_an_opening() {
    let (_runtime, manager, open, _service) = opening_with(Box::new(|_| {}), |manager| {
        *manager.spare.window.borrow_mut() = Some(ViewerWindow::new().unwrap());
        manager.release_spare_window();
        assert!(manager.spare.window.borrow().is_none());
    });
    assert!(
        !open.closed.get(),
        "an opening without a spare builds its own component"
    );
    assert!(
        !manager.spare.timer.running(),
        "a released spare schedules no refill"
    );
    manager.prepare_spare_window();
    assert!(manager.spare.timer.running());
    manager.close_all();
}

#[test]
fn recent_input_defers_the_spare_until_the_application_is_quiet() {
    let spare = SpareWindow::new(std::rc::Weak::new());
    let now = std::time::Instant::now();
    note_user_input_at(now);
    assert_eq!(spare.attempt(now), SpareAttempt::Wait(SPARE_QUIET));
    let later = now + Duration::from_millis(1200);
    assert_eq!(
        spare.attempt(later),
        SpareAttempt::Wait(SPARE_RETRY),
        "the wait is at least the retry interval"
    );
    assert_eq!(spare.attempt(now + SPARE_QUIET), SpareAttempt::Build);
}

#[test]
fn a_closed_gate_defers_the_spare_and_attempts_are_bounded() {
    let spare = SpareWindow::new(std::rc::Weak::new());
    let open = Rc::new(Cell::new(false));
    let gate = open.clone();
    *spare.gate.borrow_mut() = Some(Rc::new(move || gate.get()));
    let now = std::time::Instant::now() + Duration::from_secs(60);
    for _ in 0..SPARE_ATTEMPTS {
        assert_eq!(spare.attempt(now), SpareAttempt::Wait(SPARE_RETRY));
    }
    assert_eq!(
        spare.attempt(now),
        SpareAttempt::Stop,
        "no endless retries until the next opening"
    );
    open.set(true);
    assert_eq!(spare.attempt(now), SpareAttempt::Build);
}

#[test]
fn an_opening_in_progress_defers_the_spare_and_a_released_spare_stops() {
    let (_runtime, manager, _open, _service) = opening(Box::new(|_| {}));
    // This viewer has not shown content yet (its capture is unavailable).
    let now = std::time::Instant::now() + Duration::from_secs(60);
    assert_eq!(manager.spare.attempt(now), SpareAttempt::Wait(SPARE_RETRY));
    manager.close_all();
    assert_eq!(manager.spare.attempt(now), SpareAttempt::Build);
    manager.release_spare_window();
    assert_eq!(manager.spare.attempt(now), SpareAttempt::Stop);
}
