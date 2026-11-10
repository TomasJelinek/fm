//! Real callbacks exercise error decisions and automatic/manual Jobs ownership.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{Model, platform::Key};
use std::{
    cell::{Cell, RefCell},
    io,
    path::Path,
    rc::Rc,
    sync::Arc,
};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    refuse_decision: Rc<Cell<bool>>,
    send: Rc<dyn Fn(Command) -> bool>,
}

impl Harness {
    fn new() -> Self {
        let ui = ui::TestUi::new();
        let path = std::env::temp_dir().join("copy errors 資料");
        let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let refuse_decision = Rc::new(Cell::new(false));
        let refused = refuse_decision.clone();
        let captured = commands.clone();
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
            if refused.get() && matches!(command, Command::DecideCopy(_)) {
                return false;
            }
            captured.borrow_mut().push(command);
            true
        });
        install(&ui.window, presenter.clone(), send.clone());
        Self {
            ui,
            presenter,
            commands,
            refuse_decision,
            send,
        }
    }

    fn apply(&mut self, event: Event) {
        self.presenter
            .borrow_mut()
            .apply_with_sender(event, self.send.as_ref());
        refresh(&self.ui.window, &self.presenter.borrow());
        self.ui.draw();
    }

    fn start(&mut self, id: u64) {
        self.apply(Event::DragDrop(DragDropEvent::DropAccepted {
            drag: None,
            token: DropToken(id),
            pane: PaneId::Right,
            item_count: 1,
            intent: TransferIntent::Copy,
            verification: None,
            destination: Arc::new(std::env::temp_dir().join("copy errors destination")),
        }));
    }

    fn progress(&mut self, id: u64, intent: TransferIntent, state: CopyOperationState) {
        let mut progress = CopyProgress::queued(OperationId(id), "destination".into(), 1, intent);
        progress.state = state;
        if state == CopyOperationState::Completed {
            progress.message = "Saved and verified".into();
        }
        self.apply(Event::Copy(CopyEvent::Progress(Arc::new(progress))));
    }

    fn failure(&mut self, id: u64, version: u64, retry: bool, skip: bool) {
        self.apply(Event::Copy(CopyEvent::Failure(Arc::new(CopyFailure {
            id: OperationId(id),
            version,
            error: FsError::from_io(
                "write copy",
                Path::new("destination/資料.txt"),
                io::Error::from(io::ErrorKind::PermissionDenied),
            ),
            source: Some("source/資料.txt".into()),
            destination: "destination/資料.txt".into(),
            stage: CopyErrorStage::Writing,
            can_retry: retry,
            can_skip: skip,
        }))));
    }
}

#[test]
fn copy_error_retry_preserves_refused_token_and_jobs_resolve_restores_deferred_error() {
    let mut h = Harness::new();
    h.start(1);
    h.failure(1, 7, true, true);
    render::snapshot(&h.ui, "copy-error-normal");
    h.ui.resize(320, 200);
    render::snapshot(&h.ui, "copy-error-compact");
    h.ui.resize(1340, 820);
    assert!(h.ui.window.get_copy_is_failure());
    assert!(
        h.ui.window
            .get_copy_body()
            .contains("\n\nSource\nsource/資料.txt")
    );
    assert!(
        h.ui.window
            .get_copy_body()
            .contains("\n\nDestination\ndestination/資料.txt")
    );
    ui::popup_item(&h.ui.window, "Error", AccessibleRole::Text);
    ui::click(&h.ui.window, 5.0, 5.0);
    assert!(
        ui::has_popup(&h.ui.window),
        "outside click cannot hide an error"
    );
    h.refuse_decision.set(true);
    ui::click_popup(&h.ui.window, "Retry", AccessibleRole::Button);
    assert_eq!(h.ui.window.get_copy_token(), "copy-error:1:7");
    assert!(ui::has_popup(&h.ui.window));
    ui::click_popup(&h.ui.window, "Decide later", AccessibleRole::Button);
    assert!(!ui::has_popup(&h.ui.window));
    assert_eq!(h.ui.window.get_focused_pane(), 0);
    assert!(h.ui.window.get_operations().row_data(0).unwrap().attention);
    refresh(&h.ui.window, &h.presenter.borrow());
    assert!(
        !ui::has_popup(&h.ui.window),
        "a refresh cannot reopen a deferred error"
    );
    h.ui.window.invoke_show_operations();
    h.ui.draw();
    ui::click_popup(&h.ui.window, "Resolve", AccessibleRole::Button);
    assert_eq!(h.ui.window.get_copy_token(), "copy-error:1:7");
    h.refuse_decision.set(false);
    ui::click_popup(&h.ui.window, "Retry", AccessibleRole::Button);
    assert!(h.commands.borrow().iter().any(|command| matches!(
        command,
        Command::DecideCopy(CopyDecision {
            id: OperationId(1),
            version: 7,
            choice: CopyChoice::Retry
        })
    )));
    h.progress(1, TransferIntent::Copy, CopyOperationState::Completed);
    assert!(
        h.ui.window.get_operations_visible(),
        "manually reopened Jobs stays open"
    );
}

#[test]
fn compact_error_keeps_retry_visible_and_clickable_below_scrolling_diagnostics() {
    let mut h = Harness::new();
    h.start(1);
    h.failure(1, 7, true, true);
    h.ui.resize(320, 200);
    let retry = ui::popup_item(&h.ui.window, "Retry", AccessibleRole::Button);
    let rect = retry.geometry();
    let origin = retry.map_to_native_window(rect.origin);
    assert!(origin.y >= 0.0 && origin.y + rect.size.height <= 200.0);
    assert!(origin.x >= 0.0 && origin.x + rect.size.width <= 320.0);
    ui::click_popup(&h.ui.window, "Retry", AccessibleRole::Button);
    assert!(h.commands.borrow().iter().any(|command| matches!(
        command,
        Command::DecideCopy(CopyDecision {
            id: OperationId(1),
            version: 7,
            choice: CopyChoice::Retry
        })
    )));
}

#[test]
fn automatic_copy_and_move_jobs_close_after_success_and_keep_history() {
    let mut h = Harness::new();
    for (id, intent) in [(1, TransferIntent::Copy), (2, TransferIntent::Move)] {
        h.start(id);
        assert!(h.ui.window.get_operations_visible());
        h.progress(id, intent, CopyOperationState::Completed);
        assert!(!ui::has_popup(&h.ui.window));
        assert!(!h.ui.window.get_operations_visible());
        assert_eq!(h.ui.window.get_operations().row_count(), id as usize);
        assert_eq!(
            h.ui.window.get_operations().row_data(0).unwrap().state,
            "Completed"
        );
    }
    h.ui.window.invoke_show_operations();
    h.ui.draw();
    assert!(ui::has_popup(&h.ui.window));
    assert_eq!(h.ui.window.get_operations().row_count(), 2);
}

#[test]
fn manual_jobs_and_non_successful_terminal_jobs_do_not_auto_close() {
    let mut h = Harness::new();
    h.ui.window.invoke_show_operations();
    h.start(1);
    h.progress(1, TransferIntent::Copy, CopyOperationState::Completed);
    assert!(h.ui.window.get_operations_visible());
    ui::key(&h.ui.window, Key::Escape);
    for (id, state) in [
        (2, CopyOperationState::Cancelled),
        (3, CopyOperationState::CompletedWithErrors),
        (4, CopyOperationState::Failed),
    ] {
        h.start(id);
        h.progress(id, TransferIntent::Copy, state);
        assert!(h.ui.window.get_operations_visible());
        ui::key(&h.ui.window, Key::Escape);
    }
}

#[test]
fn successful_auto_job_waits_for_other_active_jobs_and_does_not_hide_a_waiting_error() {
    let mut h = Harness::new();
    h.start(1);
    h.start(2);
    h.progress(1, TransferIntent::Copy, CopyOperationState::Completed);
    assert!(h.ui.window.get_operations_visible());
    h.failure(2, 1, false, false);
    ui::popup_item(&h.ui.window, "Cancel job", AccessibleRole::Button);
    h.ui.window
        .invoke_decide_copy("copy-error:2:1".into(), 8, "".into());
    assert!(
        !h.commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::DecideCopy(_)))
    );
    assert!(ui::has_popup(&h.ui.window));
    h.progress(2, TransferIntent::Copy, CopyOperationState::Failed);
    assert!(h.ui.window.get_operations_visible());
}

#[test]
fn error_skip_all_dispatches_a_separate_policy_from_name_conflict_skip_all() {
    let mut h = Harness::new();
    h.start(1);
    for (version, label) in [
        (1, "Skip"),
        (2, "Skip all this error type"),
        (3, "Cancel job"),
    ] {
        h.failure(1, version, false, true);
        ui::click_popup(&h.ui.window, label, AccessibleRole::Button);
        let commands = h.commands.borrow();
        let decision = commands
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::DecideCopy(decision) => Some(decision),
                _ => None,
            })
            .unwrap();
        assert_eq!(decision.version, version);
        assert_eq!(decision.id, OperationId(1));
        assert!(matches!(
            (&decision.choice, version),
            (CopyChoice::Skip, 1) | (CopyChoice::SkipAllErrors, 2) | (CopyChoice::Cancel, 3)
        ));
    }
}

#[test]
fn automatic_jobs_finish_after_their_owner_has_left_bounded_history() {
    let mut h = Harness::new();
    for id in 1..=24 {
        h.start(id);
    }
    // Backend events can arrive in one UI tick; trimming the owner before the
    // next refresh must not erase the automatic session's successful outcome.
    for id in 1..=24 {
        let mut progress = CopyProgress::queued(
            OperationId(id),
            "destination".into(),
            1,
            TransferIntent::Copy,
        );
        progress.state = CopyOperationState::Completed;
        h.presenter.borrow_mut().apply_with_sender(
            Event::Copy(CopyEvent::Progress(Arc::new(progress))),
            h.send.as_ref(),
        );
    }
    refresh(&h.ui.window, &h.presenter.borrow());
    h.ui.draw();
    assert_eq!(h.ui.window.get_operations().row_count(), 16);
    assert!(
        !h.ui
            .window
            .get_operations()
            .iter()
            .any(|row| row.identity == "copy-job:1")
    );
    assert!(!h.ui.window.get_operations_visible());
    assert!(!ui::has_popup(&h.ui.window));
}

#[test]
fn automatic_jobs_remember_trimmed_failure_and_cancellation_outcomes() {
    let mut h = Harness::new();
    for (offset, state) in [
        (0, CopyOperationState::Failed),
        (24, CopyOperationState::Cancelled),
    ] {
        for id in 1..=24 {
            h.start(offset + id);
        }
        for id in 1..=24 {
            h.progress(
                offset + id,
                TransferIntent::Copy,
                if id == 2 {
                    state
                } else {
                    CopyOperationState::Completed
                },
            );
        }
        let failure = format!("copy-job:{}", offset + 2);
        assert!(
            !h.ui
                .window
                .get_operations()
                .iter()
                .any(|row| row.identity.as_str() == failure.as_str())
        );
        assert!(
            h.ui.window.get_operations_visible(),
            "a pruned outcome still needs attention"
        );
        ui::key(&h.ui.window, Key::Escape);
    }
}

#[test]
fn reopened_automatic_jobs_select_current_work_and_manual_jobs_keep_selection() {
    let mut h = Harness::new();
    h.start(1);
    h.progress(1, TransferIntent::Copy, CopyOperationState::Completed);
    h.start(2);
    ui::click_popup(&h.ui.window, "Copy details", AccessibleRole::Button);
    assert!(
        h.commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::CopyText { text, .. } => Some(text.contains("\nQueued\n")),
                _ => None,
            })
            .unwrap()
    );
    h.ui.window.invoke_show_operations();
    h.ui.draw();
    ui::key(&h.ui.window, Key::DownArrow);
    h.start(3);
    ui::click_popup(&h.ui.window, "Copy details", AccessibleRole::Button);
    assert!(
        h.commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::CopyText { text, .. } => Some(text.contains("\nCompleted\n")),
                _ => None,
            })
            .unwrap()
    );
}
