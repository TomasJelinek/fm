//! What a job window shows: no close button of its own beside the native frame's,
//! how a finished job ended instead of its last progress, its errors one per row
//! with the path and message in place, and no Details button: Copy details keeps them.
use super::outcome_notices::{deletion, lists, shows};
use super::*;
use i_slint_core::{item_tree::ItemRc, items::AccessibleRole};

fn apply_deletion(h: &Harness, progress: OperationProgress) {
    h.apply(Event::Operation(OperationEvent::Progress(Arc::new(
        progress,
    ))));
}

fn buttons(window: &MinimalSoftwareWindow, label: &str) -> usize {
    modeless::accessible_items(window, AccessibleRole::Button, label)
        .into_iter()
        .filter(ItemRc::is_visible)
        .count()
}

fn details_area(window: &MinimalSoftwareWindow) -> Option<ItemRc> {
    modeless::accessible_item(window, AccessibleRole::Groupbox, "Details")
        .filter(ItemRc::is_visible)
}

fn row(h: &Harness, identity: &str) -> midnight_ui::OperationRow {
    h.main
        .get_operations()
        .iter()
        .find(|row| row.identity == identity)
        .expect("the job is listed")
}

/// The window offers no Details button and shows no details area; the job's whole
/// details stay with Copy details (here once the job ended, and in Jobs).
fn no_details(h: &Harness, window: &MinimalSoftwareWindow, identity: &str) {
    for label in ["Details", "Fewer details"] {
        assert_eq!(buttons(window, label), 0, "{identity}: {label}");
    }
    assert!(
        details_area(window).is_none(),
        "{identity}: no details area"
    );
    let job = row(h, identity);
    assert!(
        job.detail.contains(job.title.as_str()),
        "{identity}: Copy details keeps the job's details"
    );
}
#[test]
fn job_windows_have_no_close_button_beside_the_native_one() {
    let h = Harness::new();
    h.start(1);
    h.copy_progress(1, TransferIntent::Copy, CopyOperationState::Running, None);
    let progress = h.children(1).remove(0);
    assert!(shows(&progress, "Copy"), "the themed header stays");
    // The footer's Cancel operation is the only close action in the window.
    assert_eq!(buttons(&progress, "Cancel operation"), 1);
    for label in ["Close progress", "Cancel dialog"] {
        assert_eq!(buttons(&progress, label), 0, "{label}");
    }
    input::key(&progress, Key::Escape);
    h.acknowledge_confirmation(1, TransferIntent::Copy);
    h.children(1);
    assert_eq!(buttons(&progress, "Dismiss cancellation question"), 0);
    assert_eq!(buttons(&progress, "Continue"), 1);
    modeless::click(&progress, "Continue");
    h.main.invoke_show_operations();
    let jobs = h.children(2).pop().unwrap();
    assert_eq!(
        buttons(&jobs, "Close dialog"),
        0,
        "Jobs has its frame's close"
    );
    assert_eq!(buttons(&jobs, "Close"), 1);
}

#[test]
fn a_finished_delete_shows_its_outcome_and_opens_its_error_in_place() {
    let h = Harness::new();
    let mut job = deletion(93, OperationState::Running, "");
    job.total = 1;
    job.completed = 0;
    job.processed_entries = 108;
    job.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        scope: DiscoveryScope::Recursive,
        files: 24_000,
        directories: 392,
        ..Default::default()
    };
    apply_deletion(&h, job.clone());
    let progress = h.children(1).remove(0);
    // Tall enough that the whole running view is on screen.
    modeless::draw(&progress, 900, 900, "delete-running-remaining");
    assert!(shows(&progress, "Items remaining: 24284"));
    job.state = OperationState::CompletedWithErrors;
    job.failed = 1;
    let locked = Path::new("source").join("folder").join("locked.txt");
    job.current_path = Some(locked.clone());
    job.errors.push(FsError::from_io(
        "delete",
        &locked,
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Permission denied (os error 13)",
        ),
    ));
    apply_deletion(&h, job);
    assert!(Rc::ptr_eq(&progress, &h.children(1)[0]));
    assert!(shows(&progress, "Completed with errors"));
    assert!(
        !shows(&progress, "Items remaining"),
        "a finished job shows no stale remaining count"
    );
    let outcome = row(&h, "93").transfer_outcome;
    assert!(
        outcome.starts_with("Items deleted: 108 · Not deleted: 24"),
        "{outcome}"
    );
    assert!(shows(&progress, &outcome));
    assert!(
        modeless::accessible_item(&progress, AccessibleRole::Groupbox, "Deletion speed").is_none()
    );
    // The error's reason and path are listed; its whole message opens in place.
    assert!(lists(&progress, "1 error"));
    assert!(lists(&progress, "Permission denied (os error 13)"));
    assert!(!shows(&progress, "Operation: delete"));
    modeless::click(&progress, "1 error");
    h.children(1);
    assert!(
        shows(&progress, "Operation: delete"),
        "the summary opens it"
    );
    assert!(shows(&progress, locked.to_str().unwrap()));
    modeless::click(&progress, "1 error");
    h.children(1);
    assert!(
        !shows(&progress, "Operation: delete"),
        "and closes it again"
    );
    modeless::click(&progress, "Permission denied (os error 13)");
    h.children(1);
    assert!(shows(&progress, "Operation: delete"), "the row opens it");
    input::key(&progress, Key::Return);
    h.children(1);
    assert!(!shows(&progress, "Operation: delete"), "Return closes it");
    assert_eq!(buttons(&progress, "Close"), 1);
    assert_eq!(buttons(&progress, "Copy details"), 1);
    no_details(&h, &progress, "93");
}

#[test]
fn copy_move_and_delete_windows_offer_no_details_button() {
    let h = Harness::new();
    for (id, intent) in [(1, TransferIntent::Copy), (2, TransferIntent::Move)] {
        h.start(id);
        h.copy_progress(id, intent, CopyOperationState::Running, None);
        let progress = h.children(1).remove(0);
        no_details(&h, &progress, &format!("copy-job:{id}"));
        let mut failed = CopyProgress::queued(OperationId(id), "destination".into(), 1, intent);
        failed.state = CopyOperationState::CompletedWithErrors;
        failed.files_completed = 1;
        failed.failed = 1;
        failed.errors.push(FsError::from_io(
            "write",
            Path::new("destination/資料.txt"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        ));
        h.apply(Event::Copy(CopyEvent::Progress(Arc::new(failed))));
        assert!(Rc::ptr_eq(&progress, &h.children(1)[0]));
        no_details(&h, &progress, &format!("copy-job:{id}"));
        input::key(&progress, Key::Escape);
        h.children(0);
    }
    apply_deletion(&h, deletion(94, OperationState::Running, ""));
    let progress = h.children(1).remove(0);
    no_details(&h, &progress, "94");
}

#[test]
fn notes_are_counted_and_only_copy_details_lists_them() {
    let h = Harness::new();
    h.start(1);
    h.copy_progress(1, TransferIntent::Copy, CopyOperationState::Running, None);
    let progress = h.children(1).remove(0);
    let mut done = CopyProgress::queued(
        OperationId(1),
        "destination".into(),
        1,
        TransferIntent::Copy,
    );
    done.state = CopyOperationState::Completed;
    done.files_completed = 30;
    done.skipped = 1;
    let note = |index: usize| {
        format!(
            "/source/{}/link-{index} was copied as a link because its target does not exist.",
            "a long folder name".repeat(6)
        )
    };
    for index in 0..20 {
        done.add_note(note(index));
    }
    h.apply(Event::Copy(CopyEvent::Progress(Arc::new(done))));
    assert!(Rc::ptr_eq(&progress, &h.children(1)[0]));
    assert!(shows(&progress, "20 links were copied as links"));
    assert!(
        !shows(&progress, "link-0 was copied"),
        "notes are not listed"
    );
    assert!(
        row(&h, "copy-job:1").issue_summary.is_empty(),
        "notes are not errors"
    );
    no_details(&h, &progress, "copy-job:1");
    assert!(
        row(&h, "copy-job:1").detail.contains("link-0 was copied"),
        "Copy details lists them"
    );
}
