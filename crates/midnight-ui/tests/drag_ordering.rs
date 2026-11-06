//! Public clipboard and Jobs callbacks preserve admitted-copy ordering under backpressure.
#[path = "support/ui.rs"]
mod ui;

use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::Model;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    blocked: Rc<Cell<bool>>,
    send: Rc<dyn Fn(Command) -> bool>,
}

impl Harness {
    fn new() -> Self {
        let ui = ui::TestUi::new();
        let path = std::env::temp_dir().join("drop ordering");
        let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path.clone())));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let blocked = Rc::new(Cell::new(true));
        let gate = blocked.clone();
        let captured = commands.clone();
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
            if gate.get() && matches!(command, Command::AcceptDrop { .. } | Command::RejectDrop(_))
            {
                return false;
            }
            captured.borrow_mut().push(command);
            true
        });
        install(&ui.window, presenter.clone(), send.clone());
        presenter.borrow_mut().apply_with_sender(
            Event::DragDrop(DragDropEvent::DropAccepted {
                token: DropToken(1),
                pane: PaneId::Right,
                item_count: 1,
                destination: Arc::new(path),
            }),
            send.as_ref(),
        );
        refresh(&ui.window, &presenter.borrow());
        Self {
            ui,
            presenter,
            commands,
            blocked,
            send,
        }
    }

    fn paste(&self) {
        self.ui.window.invoke_file_clipboard(1, 1);
        let (request, destination) = self
            .commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::ReadFileClipboard {
                    request,
                    destination,
                } => Some((*request, destination.clone())),
                _ => None,
            })
            .unwrap();
        self.presenter.borrow_mut().apply_with_sender(
            Event::ClipboardFilesRead {
                request,
                destination,
                result: Ok(FileClipboard {
                    paths: vec![std::env::temp_dir().join("clipboard source")],
                    intent: FileClipboardIntent::Copy,
                }),
            },
            self.send.as_ref(),
        );
    }

    fn recover(&self) {
        self.blocked.set(false);
        self.presenter.borrow_mut().apply_with_sender(
            Event::LocaleLoaded(Ok(NumberLocale::default())),
            self.send.as_ref(),
        );
        refresh(&self.ui.window, &self.presenter.borrow());
    }
}

#[test]
fn clipboard_copy_cannot_overtake_an_admitted_drop_waiting_for_command_capacity() {
    let h = Harness::new();
    h.paste();
    assert!(
        !h.commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::EnqueueCopy(_))),
        "a later copy must not cross the older admitted-drop operation ID"
    );
    // A clipboard result can bypass the presenter's general trailing flush. The paste
    // submission itself must drain older admitted drops once capacity returns.
    h.blocked.set(false);
    h.paste();
    let order: Vec<_> = h
        .commands
        .borrow()
        .iter()
        .filter_map(|command| match command {
            Command::AcceptDrop { id, .. } => Some(id.0),
            Command::EnqueueCopy(request) => Some(request.id.0),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        vec![1, 2],
        "backend copy IDs must arrive in submission order"
    );
}

#[test]
fn cancelling_an_admitted_drop_rejects_its_token_before_the_copy_is_registered() {
    let h = Harness::new();
    let job = h.ui.window.get_operations().row_data(0).unwrap();
    assert!(job.cancellable);
    h.ui.window.invoke_cancel_operation(job.identity);
    let cancelled = h.ui.window.get_operations().row_data(0).unwrap();
    assert!(
        !cancelled.cancellable,
        "pending admission cancellation is terminal immediately"
    );
    assert!(cancelled.title.contains("Cancelled"));
    h.recover();
    let commands = h.commands.borrow();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::RejectDrop(DropToken(1))))
    );
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::AcceptDrop { .. } | Command::CancelCopy(_))),
        "an unregistered copy must never be accepted after the user cancels it"
    );
}

#[test]
fn locally_cancelled_admissions_keep_only_the_bounded_terminal_job_history() {
    let h = Harness::new();
    for token in 1..=20 {
        if token > 1 {
            h.presenter.borrow_mut().apply_with_sender(
                Event::DragDrop(DragDropEvent::DropAccepted {
                    token: DropToken(token),
                    pane: PaneId::Right,
                    item_count: 1,
                    destination: Arc::new(std::env::temp_dir().join("drop ordering")),
                }),
                h.send.as_ref(),
            );
            refresh(&h.ui.window, &h.presenter.borrow());
        }
        let job =
            h.ui.window
                .get_operations()
                .iter()
                .find(|job| job.cancellable)
                .unwrap();
        h.ui.window.invoke_cancel_operation(job.identity);
    }
    assert_eq!(h.ui.window.get_operations().row_count(), 16);
    assert!(
        h.ui.window
            .get_operations()
            .iter()
            .all(|job| !job.cancellable)
    );
}
