//! Retained old components and delayed cancellation never change operation authority.
use super::*;
use midnight_core::{DeleteMode, EntryKind, EntryMetadata, MutationItem, OperationPreview};
use slint::platform::{
    Platform, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::sync::Arc;

struct Boundary(Rc<Cell<Duration>>);
impl Platform for Boundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
    fn duration_since_start(&self) -> Duration {
        self.0.get()
    }
}
struct Harness {
    main: MainWindow,
    state: Rc<RefCell<Presenter>>,
    ctx: Rc<Context>,
    commands: Rc<RefCell<Vec<Command>>>,
    accepting: Rc<Cell<bool>>,
    clock: Rc<Cell<Duration>>,
}
impl Harness {
    fn new() -> Self {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(Boundary(clock.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        main.show().unwrap();
        let path = std::env::temp_dir().join("operation-lifetime");
        let state = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let accepting = Rc::new(Cell::new(true));
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new({
            let commands = commands.clone();
            let accepting = accepting.clone();
            move |command| {
                commands.borrow_mut().push(command);
                accepting.get()
            }
        });
        let closing = Rc::new(Cell::new(false));
        let ctx = Rc::new(Context {
            main: main.as_weak(),
            state: Rc::downgrade(&state),
            send: send.clone(),
            owner: DialogOwner::new(&main),
            windows: RefCell::new(Windows::default()),
            failures: Failures::new(&main, &state, &send, &closing, "fixture", cancel_prepared),
            closing,
        });
        let context = ctx.clone();
        main.on_refresh_operation_windows(move || {
            let main = context.main.upgrade().unwrap();
            let state = context.state.upgrade().unwrap();
            synchronize(&context, &main, &state.borrow());
        });
        Self {
            main,
            state,
            ctx,
            commands,
            accepting,
            clock,
        }
    }
    fn preview(&self, id: u64) -> OperationWindow {
        {
            let mut p = self.state.borrow_mut();
            p.operations.pending = Some(OperationId(id));
            p.operations.preview = Some(Arc::new(OperationPreview {
                id: OperationId(id),
                kind: MutationKind::Delete(DeleteMode::Permanent),
                files: 1,
                folders: 0,
                file_bytes: Some(1),
                sample: vec![MutationItem {
                    path: std::env::temp_dir().join("operation-lifetime/source.txt"),
                    kind: EntryKind::File,
                    metadata: EntryMetadata::default(),
                }],
            }));
        }
        crate::refresh(&self.main, &self.state.borrow());
        self.ctx
            .windows
            .borrow()
            .record
            .as_ref()
            .unwrap()
            .window
            .clone_strong()
    }
    fn tick(&self, millis: u64) {
        self.clock.set(Duration::from_millis(millis));
        slint::platform::update_timers_and_animations();
    }
}
#[test]
fn native_operation_old_component_callbacks_cannot_confirm_or_cancel_replacement() {
    std::thread::spawn(|| {
        let h = Harness::new();
        let old = h.preview(11);
        let new = h.preview(12);
        assert!(!old.window().is_visible());
        assert!(new.window().is_visible());
        old.invoke_confirm("old".into());
        old.invoke_request_dismiss();
        assert!(h.commands.borrow().is_empty());
        assert_eq!(h.state.borrow().operations.pending, Some(OperationId(12)));
        new.invoke_confirm("".into());
        assert!(matches!(
            h.commands.borrow().as_slice(),
            [Command::ConfirmOperation {
                id: OperationId(12),
                new_name: None
            }]
        ));
        assert!(!new.window().is_visible());
        h.main.hide().unwrap();
    })
    .join()
    .unwrap();
}
#[test]
fn native_operation_cancel_retry_defers_while_presenter_is_borrowed() {
    std::thread::spawn(|| {
        let h = Harness::new();
        let child = h.preview(11);
        h.accepting.set(false);
        child.invoke_request_dismiss();
        assert_eq!(h.commands.borrow().len(), 1);
        assert!(!child.get_confirm_enabled());
        let guard = h.state.borrow_mut();
        h.tick(250);
        drop(guard);
        assert_eq!(
            h.commands.borrow().len(),
            1,
            "borrowed UI state must defer, not reenter"
        );
        h.accepting.set(true);
        h.tick(500);
        assert!(matches!(
            h.commands.borrow().as_slice(),
            [
                Command::CancelOperation(OperationId(11)),
                Command::CancelOperation(OperationId(11))
            ]
        ));
        assert!(!child.window().is_visible());
        h.main.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn native_operation_background_copy_retirement_keeps_owner_item_focus() {
    std::thread::spawn(|| {
        let h = Harness::new();
        h.main.invoke_set_command("unchanged draft".into(), 3);
        let inner = i_slint_core::window::WindowInner::from_pub(h.main.window());
        let invoking = inner.focus_item.borrow().upgrade().unwrap();
        h.state.borrow_mut().start_copy(
            midnight_core::FileClipboard {
                paths: vec![std::env::temp_dir().join("operation-lifetime/source.txt")],
                intent: midnight_core::FileClipboardIntent::Move,
            },
            std::env::temp_dir(),
            &|_| panic!("opening a draft must not submit a transfer"),
        );
        crate::refresh(&h.main, &h.state.borrow());
        let child = h
            .ctx
            .windows
            .borrow()
            .record
            .as_ref()
            .unwrap()
            .window
            .clone_strong();
        h.main
            .window()
            .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(false));
        child
            .window()
            .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(false));
        let token = h
            .state
            .borrow()
            .copies
            .draft
            .as_ref()
            .unwrap()
            .token
            .clone();
        h.state.borrow_mut().cancel_copy_dialog(&token);
        crate::refresh(&h.main, &h.state.borrow());
        assert!(!child.window().is_visible());
        assert!(
            inner
                .focus_item
                .borrow()
                .upgrade()
                .is_some_and(|focused| focused == invoking),
            "retiring a background draft must not replace the owner's current input focus"
        );
        h.main.hide().unwrap();
    })
    .join()
    .unwrap();
}
