//! Operation-host failure leaves backend release explicit and bounded.
use crate::*;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{RepaintBufferType, SoftwareRenderer},
};
use std::{sync::Arc, time::Duration};

#[derive(Default)]
struct Display {
    fail_create: Cell<bool>,
    fail_show: Cell<bool>,
    attempts: Cell<usize>,
    clock: Cell<Duration>,
}
struct Adapter {
    window: slint::Window,
    renderer: SoftwareRenderer,
    display: Rc<Display>,
}
impl WindowAdapter for Adapter {
    fn window(&self) -> &slint::Window {
        &self.window
    }
    fn renderer(&self) -> &dyn slint::platform::Renderer {
        &self.renderer
    }
    fn size(&self) -> slint::PhysicalSize {
        slint::PhysicalSize::new(640, 480)
    }
    fn set_visible(&self, visible: bool) -> Result<(), PlatformError> {
        if visible && self.display.fail_show.get() {
            Err("injected operation show failure".into())
        } else {
            Ok(())
        }
    }
}
struct Boundary(Rc<Display>);
impl Platform for Boundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        self.0.attempts.set(self.0.attempts.get() + 1);
        if self.0.fail_create.get() {
            return Err("injected operation creation failure".into());
        }
        Ok(Rc::new_cyclic(|weak: &std::rc::Weak<Adapter>| Adapter {
            window: slint::Window::new(weak.clone()),
            renderer: SoftwareRenderer::new_with_repaint_buffer_type(RepaintBufferType::NewBuffer),
            display: self.0.clone(),
        }))
    }
    fn duration_since_start(&self) -> Duration {
        self.0.clock.get()
    }
}
struct Harness {
    main: MainWindow,
    state: Rc<RefCell<Presenter>>,
    display: Rc<Display>,
    accepting: Rc<Cell<bool>>,
    cancelled: Rc<RefCell<Vec<OperationId>>>,
}
impl Harness {
    fn new() -> Self {
        let display = Rc::new(Display::default());
        slint::platform::set_platform(Box::new(Boundary(display.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        main.show().unwrap();
        let root = std::env::temp_dir().join("native-operation-failure");
        let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
        let accepting = Rc::new(Cell::new(false));
        let cancelled = Rc::new(RefCell::new(Vec::new()));
        install(
            &main,
            state.clone(),
            Rc::new({
                let accepting = accepting.clone();
                let cancelled = cancelled.clone();
                move |command| {
                    if let Command::CancelOperation(id) = command {
                        cancelled.borrow_mut().push(id);
                        return accepting.get();
                    }
                    assert!(
                        !matches!(command, Command::ConfirmOperation { .. }),
                        "host failure must not confirm"
                    );
                    true
                }
            }),
        );
        Self {
            main,
            state,
            display,
            accepting,
            cancelled,
        }
    }
    fn preview(&self, id: u64) {
        let mut p = self.state.borrow_mut();
        p.operations.pending = Some(OperationId(id));
        p.operations.preview = Some(Arc::new(OperationPreview {
            id: OperationId(id),
            kind: MutationKind::Delete(DeleteMode::Permanent),
            files: 1,
            folders: 0,
            file_bytes: Some(3),
            sample: vec![MutationItem {
                path: std::env::temp_dir().join("native-operation-failure/source.txt"),
                kind: EntryKind::File,
                metadata: EntryMetadata::default(),
            }],
        }));
    }
    fn refresh(&self) {
        refresh(&self.main, &self.state.borrow());
    }
    fn tick(&self, millis: u64) {
        self.display.clock.set(Duration::from_millis(millis));
        slint::platform::update_timers_and_animations();
    }
}

#[test]
fn native_operation_failed_creation_or_show_retries_only_exact_cancellation() {
    for show in [false, true] {
        std::thread::spawn(move || {
            let h = Harness::new();
            h.preview(41);
            h.display.fail_create.set(!show);
            h.display.fail_show.set(show);
            h.refresh();
            h.tick(0);
            assert_eq!(h.cancelled.borrow().as_slice(), &[OperationId(41)]);
            assert_eq!(h.state.borrow().operations.pending, Some(OperationId(41)));
            assert!(
                !h.main.get_dialog_input_blocked(),
                "failed host must release modality"
            );
            let attempts = h.display.attempts.get();
            for _ in 0..12 {
                h.refresh();
                h.tick(0);
            }
            assert_eq!(
                h.display.attempts.get(),
                attempts,
                "failed host must not be recreated"
            );
            assert_eq!(h.cancelled.borrow().len(), 1);
            h.tick(249);
            assert_eq!(h.cancelled.borrow().len(), 1);
            h.accepting.set(true);
            h.tick(250);
            assert_eq!(
                h.cancelled.borrow().as_slice(),
                &[OperationId(41), OperationId(41)]
            );
            assert!(h.state.borrow().operations.preview.is_none());
            assert!(h.state.borrow().operations.pending.is_none());
            h.main.hide().unwrap();
        })
        .join()
        .unwrap();
    }
}

#[test]
fn native_operation_failed_old_host_cannot_cancel_replacement_preview() {
    std::thread::spawn(|| {
        let h = Harness::new();
        h.preview(41);
        h.display.fail_create.set(true);
        h.refresh();
        h.tick(0);
        assert_eq!(h.cancelled.borrow().as_slice(), &[OperationId(41)]);
        h.display.fail_create.set(false);
        h.preview(42);
        h.refresh();
        h.accepting.set(true);
        h.tick(250);
        assert_eq!(h.state.borrow().operations.pending, Some(OperationId(42)));
        assert_eq!(
            h.state.borrow().operations.preview.as_ref().unwrap().id,
            OperationId(42)
        );
        assert!(
            h.main.get_dialog_input_blocked(),
            "replacement child must remain modal"
        );
        assert_eq!(h.cancelled.borrow().as_slice(), &[OperationId(41)]);
        h.main.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn native_operation_failed_copy_creation_defers_cleanup_while_presenter_is_borrowed() {
    std::thread::spawn(|| {
        let h = Harness::new();
        h.state.borrow_mut().start_copy(
            FileClipboard {
                paths: vec![std::env::temp_dir().join("native-operation-failure/source.txt")],
                intent: FileClipboardIntent::Move,
            },
            std::env::temp_dir(),
            &|_| panic!("opening a captured Move draft never submits a transfer"),
        );
        let token = h
            .state
            .borrow()
            .copies
            .draft
            .as_ref()
            .unwrap()
            .token
            .clone();
        h.display.fail_create.set(true);
        h.refresh();
        let guard = h.state.borrow_mut();
        h.tick(0);
        drop(guard);
        assert_eq!(h.state.borrow().copies.draft.as_ref().unwrap().token, token);
        h.tick(249);
        assert!(h.state.borrow().copies.draft.is_some());
        h.tick(250);
        assert!(h.state.borrow().copies.draft.is_none());
        assert!(
            h.cancelled.borrow().is_empty(),
            "local copy drafts have no prepared operation to cancel"
        );
        assert!(!h.main.get_dialog_input_blocked());
        let attempts = h.display.attempts.get();
        for _ in 0..12 {
            h.refresh();
            h.tick(500);
        }
        assert_eq!(h.display.attempts.get(), attempts);
        h.main.hide().unwrap();
    })
    .join()
    .unwrap();
}
