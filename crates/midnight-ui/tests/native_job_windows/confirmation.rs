use super::*;

#[test]
fn progress_escape_requests_confirmation_without_cancelling() {
    let h = Harness::new();
    h.start(1);
    h.copy_progress(1, TransferIntent::Copy, CopyOperationState::Running, None);
    let progress = h.children(1).remove(0);
    h.commands.borrow_mut().clear();
    input::key(&progress, Key::Escape);
    h.children(1);
    assert!(
        !question(&progress),
        "the backend has not reached a safe checkpoint"
    );
    let token = h.acknowledge_confirmation(1, TransferIntent::Copy);
    assert_confirmation_command(&h, 1, token, CopyCancelConfirmationAction::Begin);
    h.children(1);
    assert!(question(&progress));
}

fn question(window: &MinimalSoftwareWindow) -> bool {
    modeless::accessible_item(
        window,
        i_slint_core::items::AccessibleRole::Text,
        "Cancel this operation?",
    )
    .is_some()
}

fn assert_confirmation_command(
    h: &Harness,
    id: u64,
    token: u64,
    action: CopyCancelConfirmationAction,
) {
    assert!(matches!(
        h.commands.borrow().as_slice(),
        [Command::CopyCancelConfirmation { id: target, token: actual, action: sent }]
            if target.0 == id && *actual == token && *sent == action
    ));
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
        use CopyCancelConfirmationAction::{Confirm, Dismiss};
        let intent = if id == 2 {
            TransferIntent::Move
        } else {
            TransferIntent::Copy
        };
        if let Some(mode) = deletion_mode {
            h.apply(Event::Operation(OperationEvent::Progress(Arc::new(
                deletion(id, mode),
            ))));
        } else {
            h.start(id);
            h.copy_progress(id, intent, CopyOperationState::Running, None);
        }
        let progress = h.children(1).remove(0);
        let adapters = h.attempts.get();
        h.commands.borrow_mut().clear();
        let acknowledge = || {
            h.children(1);
            let token = deletion_mode.is_none().then(|| {
                assert!(!question(&progress));
                h.acknowledge_confirmation(id, intent)
            });
            h.children(1);
            assert!(question(&progress));
            h.commands.borrow_mut().clear();
            token
        };
        let continued = |token| {
            h.children(1);
            assert!(!question(&progress), "Continue must dismiss the question");
            if let Some(token) = token {
                assert_confirmation_command(&h, id, token, Dismiss);
            } else {
                assert!(h.commands.borrow().is_empty());
            }
            h.commands.borrow_mut().clear();
        };
        // Native X opens the question in the existing adapter. Enter is safe.
        progress
            .window()
            .dispatch_event(WindowEvent::CloseRequested);
        let token = acknowledge();
        assert_eq!(
            h.attempts.get(),
            adapters,
            "confirmation must not create a native window"
        );
        input::key(&progress, Key::Return);
        continued(token);
        // Escape opens it, then Escape dismisses only the question.
        input::key(&progress, Key::Escape);
        let token = acknowledge();
        input::key(&progress, Key::Escape);
        continued(token);
        // X on the question also means Continue.
        progress
            .window()
            .dispatch_event(WindowEvent::CloseRequested);
        let token = acknowledge();
        progress
            .window()
            .dispatch_event(WindowEvent::CloseRequested);
        continued(token);
        modeless::click(&progress, "Cancel operation");
        let token = acknowledge();
        modeless::click(&progress, "Continue");
        continued(token);
        modeless::click(&progress, "Cancel operation");
        let token = acknowledge();
        modeless::click(&progress, "Cancel operation");
        if let Some(token) = token {
            assert_confirmation_command(&h, id, token, Confirm);
            h.children(1);
            h.copy_progress(id, intent, CopyOperationState::CancelRequested, None);
            h.children(1);
            input::key(&progress, Key::Escape);
        } else {
            assert!(matches!(
                h.commands.borrow().as_slice(),
                [Command::CancelOperation(target)] if target.0 == id
            ));
        }
        h.children(0);
    }
}

#[test]
fn early_copy_confirmation_waits_for_registration_and_can_be_dismissed() {
    let h = Harness::new();
    h.start(1);
    let progress = h.children(1).remove(0);
    h.commands.borrow_mut().clear();
    input::key(&progress, Key::Escape);
    h.children(1);
    assert!(!question(&progress));
    assert!(
        h.commands.borrow().is_empty(),
        "the optimistic queued row is not registered"
    );
    input::key(&progress, Key::Escape);
    h.copy_progress(1, TransferIntent::Copy, CopyOperationState::Running, None);
    h.children(1);
    assert!(
        h.commands.borrow().is_empty(),
        "a locally dismissed intent cannot start a hold"
    );

    h.start(2);
    let second = h.children(2).pop().unwrap();
    h.commands.borrow_mut().clear();
    input::key(&second, Key::Escape);
    h.copy_progress(1, TransferIntent::Copy, CopyOperationState::Running, None);
    assert!(
        h.commands.borrow().is_empty(),
        "another job cannot register this operation"
    );
    h.copy_progress(2, TransferIntent::Copy, CopyOperationState::Running, None);
    let token = h.confirmation_token(2);
    assert_confirmation_command(&h, 2, token, CopyCancelConfirmationAction::Begin);
    h.children(2);
    assert!(!question(&second));
    h.acknowledge_confirmation(2, TransferIntent::Copy);
    h.children(2);
    assert!(question(&second));
}

#[test]
fn refused_hold_actions_and_stale_ready_tokens_preserve_the_current_confirmation() {
    use CopyCancelConfirmationAction::{Begin, Dismiss};
    use i_slint_core::{accessibility::AccessibilityAction, items::AccessibleRole};

    let h = Harness::new();
    h.start(1);
    h.copy_progress(1, TransferIntent::Copy, CopyOperationState::Running, None);
    let progress = h.children(1).remove(0);
    h.commands.borrow_mut().clear();
    h.refuse.set(true);
    input::key(&progress, Key::Escape);
    h.children(1);
    assert!(!question(&progress));
    assert!(h.commands.borrow().is_empty());
    h.refuse.set(false);

    input::key(&progress, Key::Escape);
    let retired = h.acknowledge_confirmation(1, TransferIntent::Copy);
    h.children(1);
    assert!(question(&progress));
    let retired_action =
        modeless::accessible_item(&progress, AccessibleRole::Button, "Cancel operation").unwrap();
    h.commands.borrow_mut().clear();
    h.refuse.set(true);
    modeless::click(&progress, "Continue");
    h.children(1);
    assert!(question(&progress));
    assert!(h.commands.borrow().is_empty());
    h.refuse.set(false);
    modeless::click(&progress, "Continue");
    assert_confirmation_command(&h, 1, retired, Dismiss);
    h.children(1);
    h.commands.borrow_mut().clear();

    input::key(&progress, Key::Escape);
    let current = h.confirmation_token(1);
    assert!(current > retired);
    assert_confirmation_command(&h, 1, current, Begin);
    h.copy_progress(
        1,
        TransferIntent::Copy,
        CopyOperationState::Running,
        Some(retired),
    );
    h.children(1);
    assert!(
        !question(&progress),
        "a retired token cannot reopen an old question"
    );
    h.acknowledge_confirmation(1, TransferIntent::Copy);
    h.children(1);
    assert!(question(&progress));
    h.commands.borrow_mut().clear();
    retired_action.accessible_action(&AccessibilityAction::Default);
    assert!(
        h.commands.borrow().is_empty(),
        "an old button cannot confirm the new token"
    );
    assert!(question(&progress));
}

#[test]
fn cancelling_delete_exposes_close_actions_in_progress_window() {
    use i_slint_core::items::AccessibleRole;

    let h = Harness::new();
    let id = OperationId(91);
    h.apply(Event::Operation(OperationEvent::Progress(Arc::new(
        deletion(id.0, DeleteMode::Permanent),
    ))));
    let progress = h.children(1).remove(0);
    assert!(
        modeless::accessible_item(&progress, AccessibleRole::Button, "Cancel operation").is_some()
    );

    h.commands.borrow_mut().clear();
    h.main.invoke_cancel_job(id.0.to_string().into());
    h.children(1);
    assert!(
        matches!(h.commands.borrow().as_slice(), [Command::CancelOperation(actual)] if *actual == id)
    );

    for label in ["Close", "Close progress"] {
        assert!(
            modeless::accessible_item(&progress, AccessibleRole::Button, label)
                .is_some_and(|item| item.is_visible()),
            "missing visible {label} action during cancellation"
        );
    }
    assert!(
        modeless::accessible_item(&progress, AccessibleRole::Button, "Cancel operation").is_none()
    );

    h.commands.borrow_mut().clear();
    modeless::click(&progress, "Close");
    h.children(0);
    assert!(h.commands.borrow().is_empty());
}

#[test]
fn cancelling_delete_shows_one_status_with_progress_and_details() {
    use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};

    let h = Harness::new();
    let mut snapshot = deletion(93, DeleteMode::Permanent);
    snapshot.state = OperationState::CancelRequested;
    snapshot.message = "Cancelling...".into();
    h.apply(Event::Operation(OperationEvent::Progress(Arc::new(
        snapshot,
    ))));
    let progress = h.children(1).remove(0);
    let status_count = || {
        ["Cancelling", "Cancelling..."]
            .into_iter()
            .flat_map(|label| modeless::accessible_items(&progress, AccessibleRole::Text, label))
            .filter(|item| item.is_visible())
            .count()
    };
    assert_eq!(status_count(), 1);
    assert!(
        modeless::accessible_item(&progress, AccessibleRole::Text, "source/資料.txt").is_some()
    );
    assert!(
        modeless::accessible_item(&progress, AccessibleRole::Groupbox, "Deletion speed").is_some()
    );
    let meter = modeless::accessible_item(
        &progress,
        AccessibleRole::ProgressIndicator,
        "Overall progress",
    )
    .unwrap();
    let row = h
        .main
        .get_operations()
        .iter()
        .find(|row| row.identity == "93")
        .unwrap();
    assert_eq!(
        meter
            .accessible_string_property(AccessibleStringProperty::Value)
            .as_deref(),
        Some(row.summary.as_str())
    );
    let detail = row
        .detail
        .strip_prefix("Permanently delete\nCancelling\n\n")
        .and_then(|body| body.strip_suffix("\n\nCancelling..."))
        .expect("Jobs clipboard details retain their full title, state and message");
    for label in ["Close", "Close progress", "Details", "Jobs"] {
        assert!(modeless::accessible_item(&progress, AccessibleRole::Button, label).is_some());
    }
    modeless::draw(&progress, 350, 300, "delete-cancelling-polish-compact");
    assert_eq!(status_count(), 1);

    modeless::draw(&progress, 700, 500, "delete-cancelling-polish");
    modeless::click(&progress, "Details");
    modeless::draw(&progress, 700, 500, "delete-cancelling-polish-details");
    assert_eq!(status_count(), 1);
    assert!(detail.contains("Source folder\nsource"));
    assert!(detail.contains("Current item\nsource/資料.txt"));
    assert!(!detail.contains("Cancelling"));
    assert!(modeless::accessible_item(&progress, AccessibleRole::Text, detail).is_some());
}

#[test]
fn terminal_update_dismisses_question_and_retired_confirmation_cannot_cancel() {
    use i_slint_core::{accessibility::AccessibilityAction, items::AccessibleRole};
    let h = Harness::new();
    h.start(1);
    h.copy_progress(1, TransferIntent::Copy, CopyOperationState::Running, None);
    h.main.invoke_open_job_progress("copy-job:1".into()); // Manual completion stays visible.
    let progress = h.children(1).remove(0);
    input::key(&progress, Key::Escape);
    h.acknowledge_confirmation(1, TransferIntent::Copy);
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
    h.copy_progress(1, TransferIntent::Copy, CopyOperationState::Running, None);
    let old = h.children(1).remove(0);
    input::key(&old, Key::Escape);
    let retired = h.acknowledge_confirmation(1, TransferIntent::Copy);
    h.children(1);
    let old_confirm =
        modeless::accessible_item(&old, AccessibleRole::Button, "Cancel operation").unwrap();
    h.complete(1);
    h.children(0);
    h.main.invoke_open_job_progress("copy-job:1".into());
    h.children(1);
    h.start(2);
    h.copy_progress(2, TransferIntent::Copy, CopyOperationState::Running, None);
    let current = h.children(2).pop().unwrap();
    input::key(&current, Key::Escape);
    let token = h.acknowledge_confirmation(2, TransferIntent::Copy);
    assert!(token > retired);
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
    assert_confirmation_command(&h, 2, token, CopyCancelConfirmationAction::Confirm);
    h.copy_progress(
        2,
        TransferIntent::Copy,
        CopyOperationState::CancelRequested,
        None,
    );
    h.children(2);
    input::key(&current, Key::Escape);
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
    let token = h.acknowledge_confirmation(1, TransferIntent::Copy);
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
    h.commands.borrow_mut().clear();
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
    assert_confirmation_command(&h, 1, token, CopyCancelConfirmationAction::Dismiss);
    h.children(2);
}

#[test]
fn accepted_copy_cancel_updates_paused_header_and_keeps_it_cancelling_through_cleanup() {
    use i_slint_core::items::AccessibleRole;
    let h = Harness::new();
    for (id, paused, pause_requested, prior_header) in [
        (1, true, true, "Paused"),
        (2, false, true, "Pausing..."),
        (3, true, false, "Resuming..."),
    ] {
        h.start(id);
        let mut snapshot = CopyProgress::queued(
            OperationId(id),
            "destination".into(),
            1,
            TransferIntent::Copy,
        );
        snapshot.state = CopyOperationState::Running;
        snapshot.phase = CopyOperationPhase::Verifying;
        snapshot.current_file_size = Some(1000);
        snapshot.current_file_copied = 1000;
        snapshot.current_file_verified = 750;
        snapshot.verification_pass_id = 9;
        snapshot.paused = paused;
        snapshot.pause_requested = pause_requested;
        snapshot.wait_reason = Some(TransferWaitReason::Resources);
        snapshot.verification_throughput = Arc::new(OperationThroughput {
            samples: vec![ThroughputSample {
                elapsed_millis: 250,
                verification_per_second: 1024.0,
                ..Default::default()
            }]
            .into(),
            verification_per_second: 1024.0,
            ..Default::default()
        });
        h.apply(Event::Copy(CopyEvent::Progress(Arc::new(snapshot.clone()))));
        let progress = h.children(1).remove(0);
        assert!(modeless::accessible_item(&progress, AccessibleRole::Text, prior_header).is_some());
        let before = h
            .main
            .get_operations()
            .iter()
            .find(|row| row.identity == format!("copy-job:{id}"))
            .unwrap();
        h.commands.borrow_mut().clear();
        h.refuse.set(true);
        h.main.invoke_cancel_job(format!("copy-job:{id}").into());
        h.children(1);
        assert!(h.commands.borrow().is_empty());
        assert!(
            modeless::accessible_item(&progress, AccessibleRole::Text, prior_header).is_some(),
            "refused cancel must leave the header unchanged"
        );
        h.refuse.set(false);
        h.main.invoke_cancel_job(format!("copy-job:{id}").into());
        h.children(1);
        assert!(
            matches!(h.commands.borrow().as_slice(), [Command::CancelCopy(actual)] if actual.0 == id)
        );
        assert!(
            modeless::accessible_item(&progress, AccessibleRole::Text, "Cancelling").is_some(),
            "header kept stale pause flags after accepted cancel"
        );
        h.apply(Event::Copy(CopyEvent::Progress(Arc::new(snapshot))));
        h.children(1);
        assert!(
            modeless::accessible_item(&progress, AccessibleRole::Text, "Cancelling").is_some(),
            "late nonterminal progress restored a stale header"
        );
        let after = h
            .main
            .get_operations()
            .iter()
            .find(|row| row.identity == format!("copy-job:{id}"))
            .unwrap();
        assert!(after.active && !after.paused && !after.pause_requested);
        assert_eq!(after.file_progress, before.file_progress);
        assert_eq!(after.file_progress_label, before.file_progress_label);
        assert_eq!(after.speed_chart, before.speed_chart);
        h.complete(id);
        h.children(0);
    }
}
