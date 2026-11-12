//! Retired embedded occurrences and delayed cancellation never change operation authority.
use super::*;
use midnight_core::{DeleteMode, EntryKind, EntryMetadata, MutationItem, OperationPreview};
use slint::platform::{
    Platform, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::sync::Arc;

struct Boundary {
    clock: Rc<Cell<Duration>>,
    adapters: Rc<Cell<usize>>,
}
impl Platform for Boundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        self.adapters.set(self.adapters.get() + 1);
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}
struct Harness {
    main: MainWindow,
    state: Rc<RefCell<Presenter>>,
    ctx: Rc<Context>,
    commands: Rc<RefCell<Vec<Command>>>,
    accepting: Rc<Cell<bool>>,
    clock: Rc<Cell<Duration>>,
    adapters: Rc<Cell<usize>>,
}
impl Harness {
    fn new() -> Self {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        let adapters = Rc::new(Cell::new(0));
        slint::platform::set_platform(Box::new(Boundary {
            clock: clock.clone(),
            adapters: adapters.clone(),
        }))
        .unwrap();
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
            adapters,
        }
    }
    fn preview(&self, id: u64) -> Rc<Record> {
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
        slint::platform::update_timers_and_animations();
        assert_eq!(
            self.adapters.get(),
            1,
            "confirmation created a native adapter"
        );
        self.ctx.windows.borrow().record.as_ref().unwrap().clone()
    }
    fn tick(&self, millis: u64) {
        self.clock.set(Duration::from_millis(millis));
        slint::platform::update_timers_and_animations();
    }
}
#[test]
fn embedded_operation_old_occurrence_cannot_confirm_or_cancel_replacement() {
    std::thread::spawn(|| {
        let h = Harness::new();
        let old = h.preview(11);
        let new = h.preview(12);
        assert!(current(&h.ctx, old.session).is_none());
        assert!(current(&h.ctx, new.session).is_some());
        assert!(h.main.global::<OperationPresentation>().get_active());
        confirm(&h.ctx, old.session, "old".into());
        dismiss(&h.ctx, old.session);
        let form = h.main.global::<OperationPresentation>();
        form.invoke_confirm(old.token.clone(), "stale popup".into());
        form.invoke_request_dismiss(old.token.clone());
        assert!(h.commands.borrow().is_empty());
        assert_eq!(h.state.borrow().operations.pending, Some(OperationId(12)));
        h.main.global::<OperationPresentation>().invoke_confirm(
            h.main.global::<OperationPresentation>().get_token(),
            "".into(),
        );
        assert!(matches!(
            h.commands.borrow().as_slice(),
            [Command::ConfirmOperation {
                id: OperationId(12),
                new_name: None
            }]
        ));
        assert!(!h.main.global::<OperationPresentation>().get_active());
        h.main.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn embedded_operation_retained_popup_controls_cannot_submit_replacement() {
    use i_slint_core::{
        accessibility::{AccessibilityAction, AccessibleStringProperty},
        item_tree::ItemRc,
        items::AccessibleRole,
        window::WindowInner,
    };

    fn find(item: ItemRc, label: &str) -> Option<ItemRc> {
        if item.accessible_role() == AccessibleRole::Button
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|text| text == label)
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(found) = find(item, label) {
                return Some(found);
            }
        }
        None
    }

    std::thread::spawn(|| {
        let h = Harness::new();
        h.preview(11);
        let popup = WindowInner::from_pub(h.main.window())
            .active_popups()
            .last()
            .unwrap()
            .component
            .clone();
        let old_confirm = find(ItemRc::new_root(popup.clone()), "Permanently delete").unwrap();
        let old_cancel = find(ItemRc::new_root(popup), "Cancel").unwrap();
        h.preview(12);
        old_confirm.accessible_action(&AccessibilityAction::Default);
        old_cancel.accessible_action(&AccessibilityAction::Default);
        assert!(h.commands.borrow().is_empty());
        assert_eq!(h.state.borrow().operations.pending, Some(OperationId(12)));
        assert!(h.main.global::<OperationPresentation>().get_active());
        let form = h.main.global::<OperationPresentation>();
        form.invoke_request_dismiss(form.get_token());
        assert!(matches!(
            h.commands.borrow().as_slice(),
            [Command::CancelOperation(OperationId(12))]
        ));
        h.main.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn embedded_operation_cancel_retry_defers_while_presenter_is_borrowed() {
    std::thread::spawn(|| {
        let h = Harness::new();
        h.preview(11);
        h.accepting.set(false);
        h.main
            .global::<OperationPresentation>()
            .invoke_request_dismiss(h.main.global::<OperationPresentation>().get_token());
        assert_eq!(h.commands.borrow().len(), 1);
        assert!(
            !h.main
                .global::<OperationPresentation>()
                .get_confirm_enabled()
        );
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
        assert!(!h.main.global::<OperationPresentation>().get_active());
        h.main.hide().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn embedded_operation_background_copy_retirement_keeps_owner_item_focus() {
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
        slint::platform::update_timers_and_animations();
        assert!(h.main.global::<OperationPresentation>().get_active());
        assert_eq!(h.adapters.get(), 1);
        h.main
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
        h.tick(0);
        assert!(!h.main.global::<OperationPresentation>().get_active());
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
