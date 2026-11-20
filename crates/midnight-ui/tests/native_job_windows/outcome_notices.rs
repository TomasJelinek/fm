//! A Copy, Move or Delete job's own open window shows how it ended (cancelled, with
//! errors or completed), so the main window adds no notice then. It adds one only
//! when no window shows the outcome.
use super::*;
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};

/// Whether a visible text of `window` contains `needle`.
pub(super) fn shows(window: &MinimalSoftwareWindow, needle: &str) -> bool {
    fn find(item: ItemRc, needle: &str) -> bool {
        if item.is_visible()
            && item.accessible_role() == AccessibleRole::Text
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|text| text.contains(needle))
        {
            return true;
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if find(item, needle) {
                return true;
            }
        }
        false
    }
    find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        needle,
    )
}

/// Whether `window` shows a button (an error count or an error row) with this label.
pub(super) fn lists(window: &MinimalSoftwareWindow, label: &str) -> bool {
    modeless::accessible_items(window, AccessibleRole::Button, label)
        .iter()
        .any(ItemRc::is_visible)
}

/// The notice the main window shows (title, body), if any.
fn notice(h: &Harness) -> Option<(String, String)> {
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
    }
    let notice = h.main.global::<NoticePresentation>();
    (!notice.get_token().is_empty()).then(|| {
        (
            notice.get_title().to_string(),
            notice.get_body().to_string(),
        )
    })
}

fn ended(id: u64, intent: TransferIntent, state: CopyOperationState) -> CopyProgress {
    let mut progress = CopyProgress::queued(OperationId(id), "destination".into(), 1, intent);
    progress.state = state;
    progress
}

fn apply_transfer(h: &Harness, progress: CopyProgress) {
    h.apply(Event::Copy(CopyEvent::Progress(Arc::new(progress))));
}

fn denied(path: &str) -> FsError {
    FsError::from_io(
        "write",
        Path::new(path),
        io::Error::from(io::ErrorKind::PermissionDenied),
    )
}

pub(super) fn deletion(id: u64, state: OperationState, message: &str) -> OperationProgress {
    OperationProgress {
        id: OperationId(id),
        kind: MutationKind::Delete(DeleteMode::Permanent),
        state,
        pause_requested: false,
        paused: false,
        discovery: Default::default(),
        total: 2,
        completed: 1,
        failed: 0,
        processed_entries: 1,
        throughput: Arc::default(),
        current_path: Some("source/資料.txt".into()),
        message: message.into(),
        errors: Vec::new(),
        source_directory: "source".into(),
        renamed_to: None,
    }
}

fn apply_deletion(h: &Harness, progress: OperationProgress) {
    h.apply(Event::Operation(OperationEvent::Progress(Arc::new(
        progress,
    ))));
}

#[test]
fn an_open_copy_or_move_window_shows_its_cancellation_without_a_notice() {
    let h = Harness::new();
    for (id, intent, said) in [
        (
            1,
            TransferIntent::Copy,
            "Copy cancelled; files already copied are kept.",
        ),
        (
            2,
            TransferIntent::Move,
            "Move cancelled; completed moves remain, and files already copied to the destination are kept.",
        ),
    ] {
        h.start(id);
        h.copy_progress(id, intent, CopyOperationState::Running, None);
        let progress = h.children(1).remove(0);
        let generation = h.main.get_message_generation();
        let mut cancelled = ended(id, intent, CopyOperationState::Cancelled);
        cancelled.files_completed = 3;
        cancelled.source_files_removed = 3;
        cancelled.message = said.into();
        apply_transfer(&h, cancelled);
        assert!(
            Rc::ptr_eq(&progress, &h.children(1)[0]),
            "the cancelled job keeps its progress window"
        );
        assert!(shows(&progress, "Cancelled"));
        assert!(
            shows(&progress, said),
            "{intent:?}: the progress window shows the outcome"
        );
        assert_eq!(
            h.main.get_message_generation(),
            generation,
            "{intent:?}: no notice repeats the outcome"
        );
        assert_eq!(notice(&h), None);
        input::key(&progress, Key::Escape);
        h.children(0);
        assert_eq!(notice(&h), None, "closing the window reports nothing more");
    }
}

#[test]
fn errors_and_completions_shown_in_an_open_window_add_no_notice() {
    let h = Harness::new();
    for (id, intent, state) in [
        (
            1,
            TransferIntent::Copy,
            CopyOperationState::CompletedWithErrors,
        ),
        (2, TransferIntent::Move, CopyOperationState::Failed),
    ] {
        h.start(id);
        h.copy_progress(id, intent, CopyOperationState::Running, None);
        let progress = h.children(1).remove(0);
        let generation = h.main.get_message_generation();
        let mut failed = ended(id, intent, state);
        failed.failed = 1;
        failed.errors.push(denied("destination/資料/locked.txt"));
        failed.message = failed.errors[0].to_string();
        apply_transfer(&h, failed);
        assert!(Rc::ptr_eq(&progress, &h.children(1)[0]));
        assert!(
            lists(&progress, "1 error"),
            "{intent:?}: the window lists it"
        );
        assert_eq!(
            h.main.get_message_generation(),
            generation,
            "{intent:?}: no notice repeats the errors"
        );
        assert_eq!(notice(&h), None);
        input::key(&progress, Key::Escape);
        h.children(0);
    }

    let mut running = deletion(91, OperationState::Running, "");
    apply_deletion(&h, running.clone());
    let progress = h.children(1).remove(0);
    let generation = h.main.get_message_generation();
    running.state = OperationState::CompletedWithErrors;
    running.failed = 1;
    running.errors.push(denied("source/資料.txt"));
    running.message = running.errors[0].to_string();
    apply_deletion(&h, running);
    assert!(Rc::ptr_eq(&progress, &h.children(1)[0]));
    assert!(lists(&progress, "1 error"));
    assert_eq!(h.main.get_message_generation(), generation);
    assert_eq!(notice(&h), None);
    input::key(&progress, Key::Escape);
    h.children(0);

    // A window opened from Jobs stays open when its job completes.
    apply_deletion(&h, deletion(92, OperationState::Running, ""));
    let progress = h.children(1).remove(0);
    h.main.invoke_open_job_progress("92".into());
    assert!(Rc::ptr_eq(&progress, &h.children(1)[0]));
    let generation = h.main.get_message_generation();
    let mut completed = deletion(92, OperationState::Completed, "Deleted 2 items.");
    completed.completed = 2;
    apply_deletion(&h, completed);
    assert!(Rc::ptr_eq(&progress, &h.children(1)[0]));
    assert!(shows(&progress, "Completed"));
    assert_eq!(h.main.get_message_generation(), generation);
    assert_eq!(notice(&h), None);
}

#[test]
fn a_cancellation_whose_progress_window_was_closed_is_reported_as_a_notice() {
    let h = Harness::new();
    h.start(1);
    h.copy_progress(
        1,
        TransferIntent::Copy,
        CopyOperationState::CancelRequested,
        None,
    );
    let progress = h.children(1).remove(0);
    // A cancelling job's window closes without another question.
    input::key(&progress, Key::Escape);
    h.children(0);
    let message = "Copy cancelled; files already copied are kept.";
    let mut cancelled = ended(1, TransferIntent::Copy, CopyOperationState::Cancelled);
    cancelled.message = message.into();
    apply_transfer(&h, cancelled);
    // No progress window opens again; the notice is a popup of the main window.
    assert!(!progress.window().is_visible());
    let (title, body) = notice(&h).expect("the outcome is reported somewhere");
    assert_eq!(title, "Copy");
    assert!(body.contains(message), "{body:?}");
}

#[test]
fn errors_of_a_job_whose_window_was_closed_are_reported_as_a_notice() {
    let h = Harness::new();
    h.start(1);
    h.copy_progress(
        1,
        TransferIntent::Copy,
        CopyOperationState::CancelRequested,
        None,
    );
    let progress = h.children(1).remove(0);
    input::key(&progress, Key::Escape);
    h.children(0);
    let mut failed = ended(1, TransferIntent::Copy, CopyOperationState::Failed);
    failed.failed = 1;
    failed.errors.push(denied("destination/資料/locked.txt"));
    apply_transfer(&h, failed);
    assert!(!progress.window().is_visible());
    let (title, body) = notice(&h).expect("errors are reported somewhere");
    assert_eq!(title, "Copy failed");
    assert!(body.contains("locked.txt"), "{body:?}");
}

#[test]
fn a_cancelled_delete_follows_the_same_rule() {
    let h = Harness::new();
    let message = "Some changes were already completed and have not been undone.";
    // Cancelled from Jobs: its progress window stays open and shows it.
    apply_deletion(&h, deletion(91, OperationState::Running, ""));
    let progress = h.children(1).remove(0);
    h.main.invoke_cancel_job("91".into());
    h.children(1);
    let generation = h.main.get_message_generation();
    apply_deletion(&h, deletion(91, OperationState::Cancelled, message));
    assert!(Rc::ptr_eq(&progress, &h.children(1)[0]));
    assert!(shows(&progress, "Cancelled"));
    assert_eq!(h.main.get_message_generation(), generation);
    assert_eq!(notice(&h), None);
    input::key(&progress, Key::Escape);
    h.children(0);

    // Cancel operation in the window keeps it open too, as Copy does, so it shows
    // how the job ended and no notice follows.
    apply_deletion(&h, deletion(92, OperationState::Running, ""));
    let progress = h.children(1).remove(0);
    modeless::click(&progress, "Cancel operation");
    h.children(1);
    modeless::click(&progress, "Cancel operation");
    assert!(
        h.commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::CancelOperation(OperationId(92))))
    );
    assert!(Rc::ptr_eq(&progress, &h.children(1)[0]));
    let generation = h.main.get_message_generation();
    apply_deletion(&h, deletion(92, OperationState::Cancelled, message));
    assert!(Rc::ptr_eq(&progress, &h.children(1)[0]));
    assert!(shows(
        &progress,
        "Delete cancelled; items already deleted are not restored."
    ));
    assert_eq!(h.main.get_message_generation(), generation);
    assert_eq!(notice(&h), None);
    input::key(&progress, Key::Escape);
    h.children(0);
    assert_eq!(notice(&h), None);
}
