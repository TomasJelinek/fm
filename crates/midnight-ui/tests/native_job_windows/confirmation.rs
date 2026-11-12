use super::*;

#[test]
fn progress_escape_requests_confirmation_without_cancelling() {
    let h = Harness::new();
    h.start(1);
    let progress = h.children(1).remove(0);
    h.commands.borrow_mut().clear();
    input::key(&progress, Key::Escape);
    assert!(
        h.commands.borrow().is_empty(),
        "Escape must ask before cancelling the job"
    );
    h.children(1);
    assert!(
        modeless::accessible_item(
            &progress,
            i_slint_core::items::AccessibleRole::Button,
            "Continue",
        )
        .is_some()
    );
}

fn question(window: &MinimalSoftwareWindow) -> bool {
    modeless::accessible_item(
        window,
        i_slint_core::items::AccessibleRole::Button,
        "Continue",
    )
    .is_some()
}

fn deletion(id: u64, mode: DeleteMode) -> OperationProgress {
    OperationProgress {
        id: OperationId(id),
        kind: MutationKind::Delete(mode),
        state: OperationState::Queued,
        pause_requested: false,
        paused: false,
        discovery: Default::default(),
        total: 1,
        completed: 0,
        failed: 0,
        processed_entries: 0,
        throughput: Arc::default(),
        current_path: Some("source/資料.txt".into()),
        message: "Queued".into(),
        errors: Vec::new(),
        source_directory: "source".into(),
        renamed_to: None,
    }
}

#[test]
fn copy_move_delete_and_trash_ask_on_both_close_routes_and_continue_is_default() {
    let h = Harness::new();
    for (id, deletion_mode) in [
        (1, None),
        (2, None),
        (91, Some(DeleteMode::Permanent)),
        (92, Some(DeleteMode::Trash)),
    ] {
        if let Some(mode) = deletion_mode {
            h.apply(Event::Operation(OperationEvent::Progress(Arc::new(
                deletion(id, mode),
            ))));
        } else {
            h.start(id);
            if id == 2 {
                h.apply(Event::Copy(CopyEvent::Progress(Arc::new(
                    CopyProgress::queued(
                        OperationId(id),
                        "destination".into(),
                        1,
                        TransferIntent::Move,
                    ),
                ))));
            }
        }
        let progress = h.children(1).remove(0);
        let adapters = h.attempts.get();
        h.commands.borrow_mut().clear();
        // Native X opens the question in the existing adapter. Enter is safe.
        progress
            .window()
            .dispatch_event(WindowEvent::CloseRequested);
        h.children(1);
        assert!(question(&progress));
        assert_eq!(
            h.attempts.get(),
            adapters,
            "confirmation must not create a native window"
        );
        input::key(&progress, Key::Return);
        h.children(1);
        assert!(
            !question(&progress),
            "Continue must own initial keyboard focus"
        );
        assert!(h.commands.borrow().is_empty());
        // Escape opens it, then Escape dismisses only the question.
        input::key(&progress, Key::Escape);
        h.children(1);
        assert!(question(&progress));
        input::key(&progress, Key::Escape);
        h.children(1);
        assert!(!question(&progress));
        assert!(h.commands.borrow().is_empty());
        // X on the question also means Continue.
        progress
            .window()
            .dispatch_event(WindowEvent::CloseRequested);
        h.children(1);
        progress
            .window()
            .dispatch_event(WindowEvent::CloseRequested);
        h.children(1);
        assert!(!question(&progress));
        assert!(h.commands.borrow().is_empty());
        modeless::click(&progress, "Cancel operation");
        h.children(1);
        assert!(question(&progress));
        modeless::click(&progress, "Continue");
        h.children(1);
        assert!(!question(&progress));
        assert!(h.commands.borrow().is_empty());
        modeless::click(&progress, "Cancel operation");
        h.children(1);
        modeless::click(&progress, "Cancel operation");
        h.children(0);
        match h.commands.borrow().as_slice() {
            [Command::CancelCopy(actual)] if deletion_mode.is_none() => {
                assert_eq!(*actual, OperationId(id))
            }
            [Command::CancelOperation(actual)] if deletion_mode.is_some() => {
                assert_eq!(*actual, OperationId(id))
            }
            commands => panic!("unexpected cancellation for {id}: {commands:?}"),
        }
    }
}

#[test]
fn terminal_update_dismisses_question_and_retired_confirmation_cannot_cancel() {
    use i_slint_core::{accessibility::AccessibilityAction, items::AccessibleRole};
    let h = Harness::new();
    h.start(1);
    h.main.invoke_open_job_progress("copy-job:1".into()); // Manual completion stays visible.
    let progress = h.children(1).remove(0);
    input::key(&progress, Key::Escape);
    h.children(1);
    let confirm =
        modeless::accessible_item(&progress, AccessibleRole::Button, "Cancel operation").unwrap();
    h.commands.borrow_mut().clear();
    h.complete(1);
    h.children(1);
    assert!(!question(&progress));
    confirm.accessible_action(&AccessibilityAction::Default);
    assert!(
        h.commands.borrow().is_empty(),
        "completion wins over pending confirmation"
    );
    input::key(&progress, Key::Escape);
    h.children(0);
}

#[test]
fn old_confirmation_cannot_close_a_reopened_receipt_or_cancel_a_new_operation() {
    use i_slint_core::{accessibility::AccessibilityAction, items::AccessibleRole};
    let h = Harness::new();
    h.start(1);
    let old = h.children(1).remove(0);
    input::key(&old, Key::Escape);
    h.children(1);
    let old_confirm =
        modeless::accessible_item(&old, AccessibleRole::Button, "Cancel operation").unwrap();
    h.complete(1);
    h.children(0);
    h.main.invoke_open_job_progress("copy-job:1".into());
    h.children(1);
    h.start(2);
    let current = h.children(2).pop().unwrap();
    input::key(&current, Key::Escape);
    h.children(2);
    h.commands.borrow_mut().clear();
    old_confirm.accessible_action(&AccessibilityAction::Default);
    old.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(
        h.commands.borrow().is_empty(),
        "retired session cannot cancel the live job"
    );
    h.children(2);
    assert!(
        question(&current),
        "retired session cannot dismiss the current question"
    );
    modeless::click(&current, "Cancel operation");
    assert!(matches!(
        h.commands.borrow().as_slice(),
        [Command::CancelCopy(OperationId(2))]
    ));
    h.children(1);
}

#[test]
fn question_preserves_pause_and_leaves_other_operations_interactive() {
    use i_slint_core::{accessibility::AccessibilityAction, items::AccessibleRole};
    let h = Harness::new();
    h.start(1);
    let mut paused = CopyProgress::queued(
        OperationId(1),
        "destination".into(),
        1,
        TransferIntent::Copy,
    );
    paused.state = CopyOperationState::Running;
    paused.pause_requested = true;
    paused.paused = true;
    h.apply(Event::Copy(CopyEvent::Progress(Arc::new(paused))));
    let first = h.children(1).remove(0);
    let old_resume = modeless::accessible_item(&first, AccessibleRole::Button, "Resume").unwrap();
    input::key(&first, Key::Escape);
    h.start(2);
    let second = h.children(2).pop().unwrap();
    h.commands.borrow_mut().clear();
    old_resume.accessible_action(&AccessibilityAction::Default);
    assert!(
        h.commands.borrow().is_empty(),
        "question must fence its underlying controls"
    );
    modeless::click(&second, "Pause");
    assert!(matches!(
        h.commands.borrow().as_slice(),
        [Command::SetCopyPaused {
            id: OperationId(2),
            paused: true
        }]
    ));
    modeless::click(&first, "Continue");
    let first_row = h
        .main
        .get_operations()
        .iter()
        .find(|row| row.identity == "copy-job:1")
        .unwrap();
    assert!(
        first_row.paused && first_row.pause_requested,
        "Continue must not resume paused work"
    );
    h.children(2);
}
