use super::*;
use midnight_core::{
    CopyClosureReceipt, MutationKind, OperationDiscovery, OperationId, TransferRecovery,
};
use slint::Model;
use std::{io, path::PathBuf};

fn locale() -> NumberLocale {
    NumberLocale {
        decimal_separator: ".".into(),
        group_separator: ",".into(),
        grouping: vec![3],
        repeat_last_group: true,
    }
}

fn queued(intent: TransferIntent) -> CopyProgress {
    CopyProgress::queued(OperationId(7), "destination".into(), 1, intent)
}

fn deleting(mode: DeleteMode, state: OperationState) -> OperationProgress {
    OperationProgress {
        id: OperationId(8),
        kind: MutationKind::Delete(mode),
        state,
        pause_requested: false,
        paused: false,
        total: 1,
        completed: 0,
        failed: 0,
        processed_entries: 0,
        discovery: OperationDiscovery::default(),
        throughput: Default::default(),
        current_path: None,
        message: String::new(),
        errors: Vec::new(),
        source_directory: "source".into(),
        renamed_to: None,
    }
}

fn rows(issues: &Issues) -> Vec<JobIssue> {
    issues.rows.iter().collect()
}

#[test]
fn a_cancelled_copy_counts_what_it_did_not_copy_and_lists_no_notes() {
    let mut p = queued(TransferIntent::Copy);
    p.state = CopyOperationState::Cancelled;
    p.files_completed = 23_956;
    p.completed_bytes = 4_000;
    p.closure = Some(CopyClosureReceipt {
        unfinished_roots: 1,
        uncommitted_files: Some(99_875),
        uncommitted_bytes: Some(6_000),
        omitted_error_details: 0,
    });
    for name in ["dangling", "to-self"] {
        p.add_note(format!(
            "/source/links/{name} was copied as a link because its target does not exist."
        ));
    }
    p.note_count = 12;
    let text = transfer(&p, false, &locale());
    assert_eq!(
        text,
        "Copy cancelled; files already copied are kept.\n\
         Files copied: 23,956 · Not copied: 99,875\n\
         Bytes copied: 4,000 · Not copied: 6,000\n\
         12 links were copied as links; Copy details says why."
    );
    let issues = transfer_issues(&p, &locale());
    assert!(issues.summary.is_empty() && rows(&issues).is_empty());
}

#[test]
fn a_cancelled_transfer_that_did_nothing_says_so() {
    for (intent, said) in [
        (TransferIntent::Copy, "Copy cancelled; nothing was copied."),
        (TransferIntent::Move, "Move cancelled; nothing was moved."),
    ] {
        let mut p = queued(intent);
        p.state = CopyOperationState::Cancelled;
        p.closure = Some(CopyClosureReceipt {
            unfinished_roots: 1,
            uncommitted_files: None,
            uncommitted_bytes: None,
            omitted_error_details: 0,
        });
        let verb = if intent == TransferIntent::Move {
            "moved"
        } else {
            "copied"
        };
        assert_eq!(
            transfer(&p, false, &locale()),
            format!("{said}\nSelected items {verb}: 0 · Not {verb}: 1")
        );
    }
}

#[test]
fn a_move_counts_files_whose_source_is_gone_and_lists_kept_sources() {
    let mut p = queued(TransferIntent::Move);
    p.state = CopyOperationState::CompletedWithErrors;
    p.files_completed = 5;
    p.source_files_removed = 4;
    p.completed_bytes = 500;
    p.failed = 1;
    p.errors.push(FsError::from_io(
        "remove source",
        &PathBuf::from("source/資料/locked.txt"),
        io::Error::new(io::ErrorKind::PermissionDenied, "Access denied"),
    ));
    p.source_residual_count = 1;
    p.source_residuals.push(MoveSourceResidual {
        source: "source/資料/kept.txt".into(),
        temporary: TransferRecovery {
            path: "source/.midnight-move-1/kept.txt".into(),
            identity: None,
        },
        matched_source: true,
    });
    p.closure = Some(CopyClosureReceipt {
        unfinished_roots: 1,
        uncommitted_files: Some(2),
        uncommitted_bytes: Some(200),
        omitted_error_details: 0,
    });
    // A file published at the destination whose source stayed is not moved, and
    // its bytes would make a byte count claim too much.
    assert_eq!(
        transfer(&p, true, &locale()),
        "Files moved: 4 · Not moved: 3"
    );
    let issues = transfer_issues(&p, &locale());
    assert_eq!(
        issues.summary,
        "1 error · 1 source kept in a temporary location"
    );
    let rows = rows(&issues);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].title, "Access denied");
    assert_eq!(rows[0].path, "source/資料/locked.txt");
    assert_eq!(rows[0].path_tail, "/locked.txt");
    assert_eq!(rows[0].detail, "Access denied\n\nOperation: remove source");
    assert_eq!(
        rows[1].title,
        "Restoring the source failed; the original is kept here"
    );
    assert_eq!(rows[1].path, "source/.midnight-move-1/kept.txt");
    assert_eq!(rows[1].detail, "Original source\nsource/資料/kept.txt");
}

#[test]
fn renamed_entries_and_unfinished_roots_are_counted_without_inventing_file_totals() {
    let mut p = queued(TransferIntent::Move);
    p.state = CopyOperationState::Cancelled;
    p.requested_roots = 3;
    p.completed_roots = 2;
    p.native_entries_moved = 2;
    p.closure = Some(CopyClosureReceipt {
        unfinished_roots: 1,
        uncommitted_files: Some(10),
        uncommitted_bytes: Some(10),
        omitted_error_details: 0,
    });
    assert_eq!(
        transfer(&p, false, &locale()),
        "Move cancelled; completed moves remain, and files already copied to the \
         destination are kept.\n\
         Moved by renaming: 2\n\
         Selected items moved: 2 · Not moved: 1"
    );
}

#[test]
fn a_cancelled_transfer_says_what_became_of_it_as_its_details_do() {
    for (intent, engine) in [
        (
            TransferIntent::Move,
            "Move cancelled; completed moves remain, and files already copied to the \
             destination are kept.",
        ),
        (
            TransferIntent::Copy,
            "Copy cancelled; files already copied are kept.",
        ),
        // Should the engine say it differently, the window says the same.
        (
            TransferIntent::Move,
            "Move cancelled; finished moves stay where they went.",
        ),
    ] {
        let mut p = queued(intent);
        p.state = CopyOperationState::Cancelled;
        p.files_completed = 2;
        p.source_files_removed = 1;
        // The job's message, which its details show: errors come first.
        p.message =
            format!("source/a.txt: Access denied\n{engine}\n0 entries moved by native rename");
        let text = transfer(&p, false, &locale());
        assert_eq!(text.lines().next(), Some(engine), "{intent:?}: {text}");
        assert_eq!(text.matches("cancelled").count(), 1, "{text}");
    }
}

#[test]
fn skips_recovery_and_unconfirmed_source_changes_are_counted() {
    let mut p = queued(TransferIntent::Copy);
    p.state = CopyOperationState::Completed;
    p.files_completed = 3;
    p.skipped = 2;
    p.recovery_count = 1;
    p.recovery_paths
        .push("destination/.midnight-recovery-1/a.txt".into());
    p.closure = Some(CopyClosureReceipt {
        unfinished_roots: 0,
        uncommitted_files: Some(0),
        uncommitted_bytes: Some(0),
        omitted_error_details: 0,
    });
    let text = transfer(&p, false, &locale());
    assert_eq!(
        text,
        "Files copied: 3\n\
         2 skipped items were not copied.\n\
         1 file was kept in a recovery folder; Copy details shows where."
    );
    assert!(!text.contains(".midnight-recovery"), "{text}");
    p.intent = TransferIntent::Move;
    p.skipped = 1;
    p.recovery_count = 0;
    p.source_files_removed = 3;
    p.source_delete_pending = 2;
    p.source_removal_durability_unconfirmed = 1;
    assert_eq!(
        transfer(&p, true, &locale()),
        "Files moved: 3\n\
         1 skipped item was not moved.\n\
         Source removals accepted by Windows: 2. Programs that still have them open can delay freeing their space.\n\
         Source changes the system could not confirm as saved to disk: 1"
    );
}

/// Links left out where no links are created are counted apart from the links copied as
/// links; Copy details names each. The job completed, so only an asked-for outcome shows it.
#[test]
fn a_completed_copy_counts_the_links_it_left_out() {
    let mut p = queued(TransferIntent::Copy);
    p.state = CopyOperationState::Completed;
    p.files_completed = 2;
    for name in ["to-folder", "to-file"] {
        p.add_note(format!(
            "/source/links/{name} was left out: no links are created at a network location."
        ));
    }
    p.links_left_out = 2;
    p.add_note(
        "/source/links/dangling was copied as a link because its target does not exist.".into(),
    );
    assert!(transfer(&p, false, &locale()).is_empty());
    let text = transfer(&p, true, &locale());
    assert!(
        text.contains(
            "2 links were left out: no links are created there; Copy details names each.\n\
             1 link was copied as a link; Copy details says why."
        ),
        "{text}"
    );
}

#[test]
fn running_and_cleanly_completed_transfers_have_no_outcome() {
    let mut p = queued(TransferIntent::Copy);
    p.state = CopyOperationState::Running;
    p.files_completed = 2;
    p.note_count = 3;
    assert!(transfer(&p, false, &locale()).is_empty());
    p.state = CopyOperationState::Completed;
    assert!(
        transfer(&p, false, &locale()).is_empty(),
        "notes alone are no reason to keep a window open"
    );
    assert!(!transfer(&p, true, &locale()).is_empty());
}

#[test]
fn a_permanent_delete_says_what_it_deleted_and_lists_its_error() {
    let mut p = deleting(DeleteMode::Permanent, OperationState::CompletedWithErrors);
    p.failed = 1;
    p.processed_entries = 108;
    p.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        scope: DiscoveryScope::Recursive,
        files: 24_000,
        directories: 392,
        ..Default::default()
    };
    let path = PathBuf::from("source").join("folder").join("readonly.txt");
    p.errors.push(FsError::from_io(
        "delete",
        &path,
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Permission denied (os error 13)",
        ),
    ));
    assert_eq!(
        deletion(&p, DeleteMode::Permanent, &locale()),
        "Items deleted: 108 · Not deleted: 24,284\nSelected items deleted: 0 · Not deleted: 1"
    );
    let issues = mutation_issues(&p, &locale());
    assert_eq!(issues.summary, "1 error");
    let rows = rows(&issues);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "Permission denied (os error 13)");
    assert_eq!(rows[0].path, path.to_str().unwrap());
    assert_eq!(
        rows[0].detail,
        "Permission denied (os error 13)\n\nOperation: delete"
    );
}

#[test]
fn trash_fallback_reports_recycled_and_permanently_deleted_targets_separately() {
    let mut p = deleting(DeleteMode::Permanent, OperationState::CompletedWithErrors);
    p.total = 4;
    p.completed = 3;
    p.message =
        "Moved to Trash: 1\nPermanently deleted: 2\nRemaining selected items: 1\nentry is busy"
            .into();
    assert_eq!(
        deletion(&p, DeleteMode::Permanent, &locale()),
        "Moved to Trash: 1\nPermanently deleted: 2\nRemaining selected items: 1"
    );
}

#[test]
fn a_cancelled_trash_move_says_what_stays_in_the_trash() {
    let mut p = deleting(DeleteMode::Trash, OperationState::Cancelled);
    p.total = 5;
    p.completed = 2;
    p.processed_entries = 2;
    p.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        scope: DiscoveryScope::SelectedRoots,
        files: 5,
        ..Default::default()
    };
    assert_eq!(
        deletion(&p, DeleteMode::Trash, &locale()),
        "Move to Trash cancelled; items already in the Trash stay there.\n\
         Selected items moved to the Trash: 2 · Not moved: 3"
    );
    p.completed = 0;
    p.processed_entries = 0;
    assert!(
        deletion(&p, DeleteMode::Trash, &locale())
            .starts_with("Move to Trash cancelled; nothing was moved to the Trash.\n")
    );
    p.state = OperationState::Completed;
    p.completed = 5;
    assert!(deletion(&p, DeleteMode::Trash, &locale()).is_empty());
}

#[test]
fn other_changes_keep_message_lines_their_errors_do_not_say() {
    let error = FsError::from_io(
        "rename",
        &PathBuf::from("source/old.txt"),
        io::Error::new(io::ErrorKind::AlreadyExists, "File exists"),
    );
    let mut p = deleting(DeleteMode::Permanent, OperationState::Failed);
    p.kind = MutationKind::Rename;
    p.failed = 1;
    p.message = format!("{error}\nThe original is kept in source/.recovery/old.txt");
    p.errors.push(error);
    assert_eq!(
        change(&p),
        "The original is kept in source/.recovery/old.txt"
    );
    p.state = OperationState::Running;
    assert!(change(&p).is_empty());
}

#[test]
fn error_counts_say_how_many_details_were_kept() {
    let locale = locale();
    assert_eq!(counted_list(0, 0, "error", "errors", &locale), None);
    assert_eq!(
        counted_list(1, 1, "error", "errors", &locale).as_deref(),
        Some("1 error")
    );
    assert_eq!(
        counted_list(1_020, 16, "error", "errors", &locale).as_deref(),
        Some("1,020 errors, 16 listed")
    );
    assert_eq!(
        counted_list(3, 0, "error", "errors", &locale).as_deref(),
        Some("3 errors; their details were not kept")
    );
    // A Delete counts targets with an issue; one target can report two errors.
    assert_eq!(
        counted_list(1, 2, "error", "errors", &locale).as_deref(),
        Some("2 errors")
    );
}

#[test]
fn error_rows_show_controls_as_escapes_and_keep_long_paths_whole() {
    let long = PathBuf::from("資料")
        .join("long folder/".repeat(40))
        .join("photo\u{202e}gpj.exe");
    let row = error_row(&FsError::from_io(
        "copy files",
        &long,
        io::Error::other("first line\nsecond line"),
    ));
    assert!(row.path.ends_with("photo\\u202Egpj.exe"), "{}", row.path);
    assert!(row.path.contains(&"long folder/".repeat(40)));
    // A shortened row keeps the end of the shown path in view.
    assert!(!row.path_tail.is_empty() && row.path.ends_with(row.path_tail.as_str()));
    assert_eq!(row.title, "first line");
    assert_eq!(
        row.detail,
        "first line\nsecond line\n\nOperation: copy files"
    );
    let bare = error_row(&FsError::from_io(
        "prepare",
        &PathBuf::new(),
        io::Error::other(""),
    ));
    assert_eq!(bare.title, "prepare");
    assert!(bare.path.is_empty() && bare.path_tail.is_empty());
    assert_eq!(bare.detail, "Operation: prepare");
}

#[test]
fn no_outcome_says_published() {
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        for state in [
            CopyOperationState::Cancelled,
            CopyOperationState::CompletedWithErrors,
            CopyOperationState::Failed,
            CopyOperationState::Completed,
        ] {
            let mut p = queued(intent);
            p.state = state;
            p.files_completed = 1;
            p.source_files_removed = 1;
            p.completed_bytes = 1;
            p.skipped = 1;
            p.closure = Some(CopyClosureReceipt {
                unfinished_roots: 1,
                uncommitted_files: Some(1),
                uncommitted_bytes: Some(1),
                omitted_error_details: 1,
            });
            let text = transfer(&p, true, &locale());
            assert!(!text.is_empty());
            assert!(!text.to_lowercase().contains("publish"), "{text}");
        }
    }
}
