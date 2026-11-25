use super::*;
use midnight_core::{CopyClosureReceipt, TransferRemainingWork, TransferWaitReason};
use std::{io, path::PathBuf, sync::Arc};

fn progress(id: u64) -> CopyProgress {
    CopyProgress {
        state: CopyOperationState::Running,
        phase: CopyOperationPhase::Verifying,
        files_completed: 2,
        directories_created: 3,
        copied_bytes: u64::MAX,
        verified_bytes: 123,
        current_path: Some(
            PathBuf::from("資料")
                .join("long/".repeat(100))
                .join("🦀.txt"),
        ),
        ..CopyProgress::queued(
            OperationId(id),
            "destination/資料".into(),
            1,
            TransferIntent::Copy,
        )
    }
}

#[test]
fn transfer_overview_uses_discovered_totals_and_separate_current_path_parts() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = progress(23);
    let directory = PathBuf::from("資料").join("long folder");
    value.current_path = Some(directory.join("報告.txt"));
    value.discovery = OperationDiscovery {
        state: DiscoveryState::Scanning,
        scope: DiscoveryScope::Recursive,
        files: 8,
        bytes: Some(840),
        ..Default::default()
    };
    value.completed_bytes = 200;
    value.current_file_copied = 90;
    value.files_completed = 2;

    let scanning = copy(&p, &value);
    assert!(!scanning.determinate);
    assert!(scanning.transfer_totals.is_empty());
    assert_eq!(
        scanning.transfer_directory,
        directory.to_string_lossy().as_ref()
    );
    assert_eq!(scanning.transfer_name, "報告.txt");

    value.discovery.state = DiscoveryState::Complete;
    value.phase = CopyOperationPhase::Verifying;
    value.verification_pass_id = 1;
    value.current_file_size = Some(90);
    value.current_file_verified = 9;
    let verifying = copy(&p, &value);
    // Written bytes include the current file's, as the percentage does.
    assert_eq!(
        verifying.transfer_totals,
        "Files: 2 of 8 · Bytes: 290 B of 840 B"
    );
    assert_eq!(verifying.progress_label, "34% complete");
    assert_eq!(verifying.file_progress, 0.1);

    value.phase = CopyOperationPhase::Publishing;
    let publishing = copy(&p, &value);
    assert_eq!(publishing.state, "Copying");
    assert_eq!(publishing.progress, verifying.progress);

    value.intent = TransferIntent::Move;
    value.completed_roots = 2;
    value.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        scope: DiscoveryScope::SelectedRoots,
        files: 1,
        directories: 2,
        ..Default::default()
    };
    let moved = copy(&p, &value);
    assert_eq!(moved.transfer_totals, "Selected items: 2 of 3");
    assert_eq!(moved.progress_label, "66% complete");
}

#[test]
fn cancelled_and_warning_transfers_show_their_outcome_and_list_only_their_errors() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = progress(27);
    value.requested_roots = 4;
    value.completed_roots = 1;
    value.discovery.state = DiscoveryState::Unavailable;
    value.state = CopyOperationState::Cancelled;
    value.skipped = 1;
    value.failed = 3;
    value.closure = Some(CopyClosureReceipt {
        unfinished_roots: 3,
        uncommitted_files: None,
        uncommitted_bytes: None,
        omitted_error_details: 2,
    });
    value.add_note(
        "/source/links/dangling was copied as a link because its target does not exist.".into(),
    );
    let residual = PathBuf::from("target").join("partial");
    value.errors.push(FsError::from_io(
        "cleanup",
        &residual,
        std::io::Error::other("Temporary file removal failed"),
    ));
    let cancelled = copy(&p, &value);
    let outcome = cancelled.transfer_outcome.as_str();
    assert!(outcome.starts_with("Copy cancelled; files already copied are kept.\n"));
    assert!(outcome.contains("Files copied: 2"), "{outcome}");
    assert!(outcome.contains("Selected items copied: 1 · Not copied: 3"));
    assert!(outcome.contains("1 skipped item was not copied."));
    assert!(outcome.contains("1 link was copied as a link"));
    // Errors are listed one per row, never in the outcome text, and notes not at all.
    assert!(!outcome.contains("Temporary file removal failed"));
    assert!(!outcome.contains("/source/links/dangling"));
    assert_eq!(cancelled.issue_summary, "3 errors, 1 listed");
    let issues: Vec<_> = cancelled.issues.iter().collect();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].title, "Temporary file removal failed");
    assert_eq!(issues[0].path, residual.to_string_lossy().as_ref());
    // Copy details keep everything: the error, the notes and what was not retained.
    assert!(
        cancelled
            .detail
            .contains(residual.to_string_lossy().as_ref())
    );
    assert!(cancelled.detail.contains("/source/links/dangling"));
    assert!(cancelled.detail.contains("2 more error details"));
    for text in [outcome, cancelled.detail.as_str()] {
        assert!(!text.to_lowercase().contains("publish"), "{text}");
    }

    value.state = CopyOperationState::Completed;
    value.failed = 0;
    value.errors.clear();
    value.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        scope: DiscoveryScope::Recursive,
        files: 4,
        bytes: Some(375),
        ..Default::default()
    };
    value.completed_bytes = 250;
    value.closure = Some(CopyClosureReceipt {
        unfinished_roots: 3,
        uncommitted_files: Some(2),
        uncommitted_bytes: Some(125),
        omitted_error_details: 0,
    });
    let warning = copy(&p, &value);
    assert_eq!(warning.state, "Completed with warnings");
    let outcome = warning.transfer_outcome.as_str();
    assert!(outcome.contains("1 skipped item was not copied."));
    assert!(
        outcome.contains("Files copied: 2 · Not copied: 2"),
        "{outcome}"
    );
    assert!(outcome.contains("Bytes copied: 250 · Not copied: 125"));
    assert!(!outcome.contains("Selected items"), "file totals are known");
    assert!(warning.issue_summary.is_empty());
    assert_eq!(warning.issues.row_count(), 0);
    assert!(!warning.detail.contains("more error details"));
}

#[test]
fn copy_keeps_full_source_destination_and_verification_without_inventing_a_percentage() {
    let p = Presenter::new("left".into(), "right".into());
    let progress = progress(1);
    let row = copy(&p, &progress);
    assert!(
        !row.determinate,
        "no metadata discovery result is available yet"
    );
    assert_eq!(row.progress, 0.0);
    assert!(
        row.detail.contains(
            &progress
                .current_path
                .as_ref()
                .unwrap()
                .display()
                .to_string()
        )
    );
    assert!(row.detail.contains("Destination\ndestination"));
    assert!(row.detail.contains("123 bytes verified (SHA-256)"));
    assert!(row.detail.contains("Verifying SHA-256"));
    assert!(!row.failed);
}

#[test]
fn job_details_retain_error_path_when_it_differs_from_current_file() {
    let p = Presenter::new("left".into(), "right".into());
    let mut progress = progress(2);
    progress.state = CopyOperationState::CompletedWithErrors;
    progress.failed = 1;
    progress.errors.push(FsError::from_io(
        "verify copy",
        std::path::Path::new("failed/資料/file.bin"),
        io::Error::other("Checksum mismatch"),
    ));
    let row = copy(&p, &progress);
    assert!(row.failed);
    assert!(!row.cancellable);
    assert!(row.detail.contains("Checksum mismatch\n\nPath\nfailed"));
    assert!(row.detail.contains("Operation: verify copy"));
}

#[test]
fn cancelled_transfer_keeps_cleanup_failure_and_exact_stage_path_visible() {
    let p = Presenter::new("left".into(), "right".into());
    let stage = PathBuf::from("destination/資料/.me-0123456789abcdef");
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        let mut progress = progress(6);
        progress.intent = intent;
        progress.state = CopyOperationState::Cancelled;
        progress.current_path = None;
        progress.failed = 16;
        progress.errors = (0..15)
            .map(|index| {
                FsError::from_io(
                    "read source",
                    &PathBuf::from(format!("source-{index}")),
                    io::Error::other("Earlier failure"),
                )
            })
            .collect();
        progress.errors.push(FsError::from_io(
            "cleanup copy stage",
            &stage,
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Temporary file removal failed",
            ),
        ));
        let row = copy(&p, &progress);
        assert_eq!(row.state, "Cancelled");
        assert!(
            row.failed,
            "cancellation must not hide an unsuccessful cleanup"
        );
        assert!(!row.active && !row.cancellable);
        assert!(row.detail.contains("Temporary file removal failed"));
        assert!(row.detail.contains(stage.to_str().unwrap()));
        assert!(row.summary.contains("2 files"));
        assert!(row.summary.contains("16 errors"));
    }
}

#[test]
fn copy_details_show_retained_originals_and_truthful_bounded_samples() {
    let p = Presenter::new("left".into(), "right".into());
    let mut progress = progress(3);
    progress.state = CopyOperationState::Completed;
    progress.recovery_count = 20;
    progress.recovery_paths = vec![PathBuf::from("destination/復元/original")];
    let row = copy(&p, &progress);
    assert!(row.detail.contains("20 recovery files retained"));
    assert!(row.detail.contains("destination/復元/original"));
    assert!(
        row.detail
            .contains("Additional displaced destination files")
    );
    assert!(!row.failed);
}

#[test]
fn copy_details_list_link_notes_without_turning_them_into_errors() {
    let p = Presenter::new("left".into(), "right".into());
    let mut progress = progress(4);
    progress.state = CopyOperationState::Completed;
    for name in ["dangling", "to-self"] {
        progress.add_note(format!(
            "/source/links/{name} was copied as a link because it could not be followed."
        ));
    }
    progress.note_count = 18;
    let row = copy(&p, &progress);
    assert!(
        row.detail
            .contains("\n\nNotes\n/source/links/dangling was copied as a link")
    );
    assert!(
        row.detail
            .contains("/source/links/to-self was copied as a link")
    );
    assert!(row.detail.contains("16 more notes are not shown."));
    assert!(!row.failed);
    // A clean completion with notes needs no review: no outcome and no error rows.
    assert!(row.transfer_outcome.is_empty());
    assert!(row.issue_summary.is_empty() && row.issues.row_count() == 0);
}

#[test]
fn copy_and_mutation_jobs_share_chronological_order_and_stable_identity() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.copies.progress = vec![Arc::new(progress(1)), Arc::new(progress(3))];
    p.operations.progress.push(Arc::new(OperationProgress {
        pause_requested: false,
        paused: false,
        discovery: Default::default(),
        id: OperationId(2),
        throughput: Arc::default(),
        kind: MutationKind::Rename,
        state: OperationState::Running,
        total: 1,
        completed: 0,
        failed: 0,
        processed_entries: 0,
        current_path: None,
        source_directory: "source".into(),
        renamed_to: None,
        message: String::new(),
        errors: vec![],
    }));
    let rows = rows(&p);
    assert_eq!(
        rows.iter()
            .map(|row| row.identity.as_str())
            .collect::<Vec<_>>(),
        ["copy-job:3", "rename-job:2", "copy-job:1"]
    );
    assert!(rows[1].determinate);
    assert_eq!(rows[1].progress, 0.0);
}

#[test]
fn move_reports_exceptional_original_locations_and_never_claims_source_space_was_freed() {
    let p = Presenter::new("left".into(), "right".into());
    let mut progress = progress(4);
    progress.intent = TransferIntent::Move;
    progress.phase = CopyOperationPhase::ValidatingAfterMove;
    progress.source_names_staged = 1;
    progress.source_residual_count = 1;
    progress.validation_bytes = 246;
    progress.source_residuals.push(MoveSourceResidual {
        source: "source/Žluťoučký.txt".into(),
        temporary: TransferRecovery {
            path: "source/.midnight-recovery-test/Žluťoučký.txt".into(),
            identity: None,
        },
        matched_source: true,
    });
    let row = copy(&p, &progress);
    assert_eq!(row.title, "Move");
    assert_eq!(row.state, "Checking before source removal");
    assert!(row.detail.contains("0 source files removed"));
    assert!(
        row.detail
            .contains("1 temporary source location needs attention")
    );
    assert!(row.detail.contains("Original source\nsource/Žluťoučký.txt"));
    assert!(
        row.detail
            .contains("Temporary location\nsource/.midnight-recovery-test/Žluťoučký.txt")
    );
    assert!(!row.failed);
    assert!(
        !row.determinate,
        "no metadata discovery result is available yet"
    );
    assert_eq!(row.progress, 0.0);
}

#[test]
fn move_reports_removed_sources_separately_from_retained_and_pending_storage() {
    let p = Presenter::new("left".into(), "right".into());
    let mut progress = progress(5);
    progress.intent = TransferIntent::Move;
    progress.source_files_removed = 3;
    progress.source_delete_pending = 2;
    progress.source_removal_durability_unconfirmed = 1;
    progress.source_residual_count = 1;
    let row = copy(&p, &progress);
    assert!(row.detail.contains("3 source files removed"));
    assert!(
        row.detail
            .contains("1 temporary source location needs attention")
    );
    assert!(row.detail.contains("2 source removals accepted by Windows"));
    assert!(
        row.detail
            .contains("1 source namespace change without a confirmed")
    );
}

#[test]
fn progress_distinguishes_root_completion_from_file_write_and_verification() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = progress(7);
    value.requested_roots = 4;
    value.completed_roots = 1;
    value.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        scope: DiscoveryScope::SelectedRoots,
        files: 4,
        ..Default::default()
    };
    value.phase = CopyOperationPhase::Copying;
    value.current_file_size = Some(1_000);
    value.verification_pass_id = 1;
    value.current_file_copied = 500;
    let row = copy(&p, &value);
    assert_eq!(row.progress, 0.25);
    assert_eq!(row.file_progress, 0.5);
    assert!(row.file_determinate && row.active && !row.completed);
    assert_eq!(row.progress_label, "25% complete");
    value.current_file_copied = 1_000;
    value.phase = CopyOperationPhase::Verifying;
    value.current_file_verified = 250;
    let row = copy(&p, &value);
    assert_eq!(row.file_progress, 0.25);
    assert!(row.verification_view);
    assert_eq!(row.file_progress_label, "Verifying: 250 / 1000 bytes");
    assert_eq!(row.progress, 0.25, "a written file is not yet committed");
    assert!(row.state.contains("Verifying"));
    value.current_file_size = None;
    assert!(!copy(&p, &value).file_determinate);
}

#[test]
fn discovered_totals_keep_recursive_counts_and_exact_bytes_in_copy_details() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = progress(8);
    value.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        scope: DiscoveryScope::Recursive,
        files: 23,
        directories: 4,
        bytes: Some(123_456),
        approximate: false,
        message: String::new(),
    };
    let row = copy(&p, &value);
    assert!(row.detail.contains("Discovered totals"));
    assert!(
        row.detail
            .contains("Selected items and discovered folder contents")
    );
    assert!(
        row.detail
            .contains("Files and other entries: 23\nFolders: 4")
    );
    assert!(row.detail.contains(&format!(
        "Logical file bytes: {} bytes",
        p.number_locale.format_u64(123_456)
    )));
    // One byte in the singular.
    value.discovery.bytes = Some(1);
    let row = copy(&p, &value);
    assert!(
        row.detail.contains("Logical file bytes: 1 byte ("),
        "{}",
        row.detail
    );
    value
        .discovery
        .unavailable("Source membership changed after discovery.");
    let row = copy(&p, &value);
    assert!(!row.detail.contains("Discovered totals"));
    assert!(
        row.detail
            .contains("Source membership changed after discovery.")
    );
}

/// Jobs rows and job details count in the singular for one: "1 file · 0 skipped ·
/// 1 error", not "1 files · 0 skipped · 1 errors".
#[test]
fn job_rows_and_details_count_one_in_the_singular() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = progress(31);
    value.state = CopyOperationState::CompletedWithErrors;
    value.intent = TransferIntent::Move;
    value.files_completed = 1;
    value.failed = 1;
    value.directories_created = 1;
    value.copied_bytes = 1;
    value.verified_bytes = 1;
    value.validation_bytes = 1;
    value.source_files_removed = 1;
    value.source_residual_count = 1;
    value.source_delete_pending = 1;
    value.source_removal_durability_unconfirmed = 1;
    value.recovery_count = 1;
    let row = copy(&p, &value);
    assert_eq!(row.summary, "1 file · 0 skipped · 1 error");
    for line in [
        "\n1 folder created\n1 byte copied\n1 byte verified (",
        "\n1 additional source/target byte checked\n1 source file removed\n1 temporary source location needs attention",
        "\n1 source removal accepted by Windows",
        "\n1 source namespace change without a confirmed",
        "\n1 recovery file retained",
    ] {
        assert!(row.detail.contains(line), "{line:?} in {:?}", row.detail);
    }
    value.files_completed = 2;
    value.failed = 0;
    assert_eq!(copy(&p, &value).summary, "2 files · 0 skipped · 0 errors");
    let deletion = OperationProgress {
        id: OperationId(32),
        kind: MutationKind::Delete(DeleteMode::Trash),
        state: OperationState::Completed,
        pause_requested: false,
        paused: false,
        total: 1,
        completed: 1,
        failed: 1,
        processed_entries: 1,
        current_path: None,
        source_directory: "source".into(),
        renamed_to: None,
        message: String::new(),
        errors: vec![],
        throughput: Arc::default(),
        discovery: OperationDiscovery::default(),
    };
    assert_eq!(mutation(&p, &deletion).summary, "1 / 1 target · 1 error");
}

#[test]
fn delete_details_mark_selected_root_scope_and_unknown_bytes_explicitly() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = OperationProgress {
        id: OperationId(9),
        kind: MutationKind::Delete(DeleteMode::Trash),
        state: OperationState::Running,
        pause_requested: false,
        paused: false,
        total: 3,
        completed: 0,
        failed: 0,
        processed_entries: 0,
        current_path: None,
        source_directory: "source".into(),
        renamed_to: None,
        message: String::new(),
        errors: vec![],
        throughput: Arc::default(),
        discovery: OperationDiscovery {
            state: DiscoveryState::Complete,
            scope: DiscoveryScope::SelectedRoots,
            files: 1,
            directories: 2,
            bytes: None,
            approximate: false,
            message: String::new(),
        },
    };
    let row = mutation(&p, &value);
    assert!(
        row.detail
            .contains("Selected items only; folder contents were not counted")
    );
    assert!(
        row.detail
            .contains("Files and other entries: 1\nFolders: 2")
    );
    assert!(row.detail.contains("Logical file bytes: Unknown"));
    value.discovery.state = DiscoveryState::Scanning;
    assert!(!mutation(&p, &value).detail.contains("Discovered totals"));
    value
        .discovery
        .unavailable("Directory metadata could not be read.");
    let row = mutation(&p, &value);
    assert!(!row.detail.contains("Discovered totals"));
    assert!(row.detail.contains("Directory metadata could not be read."));
}

#[test]
fn delete_progress_distinguishes_waiting_scanning_and_control_states() {
    let mut p = Presenter::new("left".into(), "right".into());
    let mut progress = OperationProgress {
        id: OperationId(41),
        kind: MutationKind::Delete(DeleteMode::Permanent),
        state: OperationState::Queued,
        pause_requested: false,
        paused: false,
        total: 3,
        completed: 0,
        failed: 0,
        processed_entries: 0,
        current_path: None,
        source_directory: "source".into(),
        renamed_to: None,
        message: String::new(),
        errors: vec![],
        throughput: Arc::default(),
        discovery: OperationDiscovery::default(),
    };
    let row = |p: &Presenter| {
        let mut jobs = rows(p);
        assert_eq!(jobs.len(), 1);
        let row = jobs.pop().unwrap();
        assert_eq!(row.identity, "41");
        row
    };
    let update = |p: &mut Presenter, progress: &OperationProgress| {
        p.apply(Event::Operation(OperationEvent::Progress(Arc::new(
            progress.clone(),
        ))));
        row(p)
    };

    let queued = update(&mut p, &progress);
    assert_eq!(queued.state, "Queued");
    assert_eq!(queued.progress_label, "Queued — waiting to start");
    assert_eq!(queued.activity_label, "Queued — waiting to start");
    assert!(queued.active && queued.waiting);
    assert!(!queued.discovering && !queued.determinate && queued.eta.is_empty());

    progress.state = OperationState::Running;
    let starting = update(&mut p, &progress);
    assert_eq!(starting.progress_label, "Preparing");
    assert!(!starting.waiting && !starting.discovering);

    progress.discovery = OperationDiscovery {
        state: DiscoveryState::Scanning,
        files: 3,
        directories: 2,
        bytes: Some(900),
        ..Default::default()
    };
    let scanning = update(&mut p, &progress);
    // A Delete counts items; the size it removes is no measure of its work.
    assert_eq!(scanning.progress_label, "Discovering: 5 items");
    assert_eq!(scanning.activity_label, "Discovering items...");
    assert!(scanning.discovering && !scanning.waiting);
    assert!(!scanning.determinate && scanning.eta.is_empty());

    p.set_job_paused("41", true, &|command| {
        matches!(
            command,
            Command::SetOperationPaused {
                id: OperationId(41),
                paused: true
            }
        )
    });
    let pausing = row(&p);
    assert_eq!(pausing.progress_label, "Pausing...");
    assert_eq!(pausing.state, "Pausing...", "the Jobs list says so too");
    assert!(pausing.pause_requested && !pausing.paused && !pausing.discovering);
    progress.pause_requested = true;
    progress.paused = true;
    let paused = update(&mut p, &progress);
    assert_eq!(paused.activity_label, "Paused");
    assert_eq!(paused.state, "Paused");
    assert!(paused.detail.contains("\nPaused\n"));
    assert!(paused.paused && !paused.discovering && paused.eta.is_empty());

    p.set_job_paused("41", false, &|command| {
        matches!(
            command,
            Command::SetOperationPaused {
                id: OperationId(41),
                paused: false
            }
        )
    });
    let resuming = row(&p);
    assert_eq!(resuming.activity_label, "Resuming...");
    assert_eq!(resuming.state, "Resuming...");
    assert!(resuming.paused && !resuming.pause_requested && !resuming.discovering);

    p.cancel_operation("41", &|command| {
        matches!(command, Command::CancelOperation(OperationId(41)))
    });
    for cancelling in [row(&p), update(&mut p, &progress)] {
        assert_eq!(cancelling.state, "Cancelling");
        assert_eq!(cancelling.progress_label, "Cancelling...");
        assert_eq!(cancelling.activity_label, "Cancelling...");
        assert!(cancelling.active && !cancelling.cancellable);
        assert!(!cancelling.discovering && !cancelling.waiting && cancelling.eta.is_empty());
        assert!(!cancelling.paused && !cancelling.pause_requested);
    }

    let mut cleanup = progress.clone();
    cleanup.state = OperationState::CancelRequested;
    cleanup.current_path = Some("source/cleanup-資料.txt".into());
    cleanup.completed = 1;
    cleanup.failed = 1;
    cleanup.processed_entries = 2;
    cleanup.discovery.state = DiscoveryState::Complete;
    cleanup.discovery.scope = DiscoveryScope::Recursive;
    cleanup.errors.push(FsError::from_io(
        "delete cleanup",
        std::path::Path::new("source/cleanup-資料.txt"),
        io::Error::new(io::ErrorKind::PermissionDenied, "Cleanup access denied"),
    ));
    let refreshed = update(&mut p, &cleanup);
    assert_eq!(refreshed.state, "Cancelling");
    assert_eq!(refreshed.current_path, "source/cleanup-資料.txt");
    assert_eq!(refreshed.summary, "1 / 3 targets · 1 error");
    assert!(refreshed.detail.contains("2 entries processed"));
    assert!(refreshed.detail.contains("Cleanup access denied"));
    assert!(refreshed.active && refreshed.failed && !refreshed.cancellable);

    for stale in [OperationState::Running, OperationState::Queued] {
        progress.state = stale;
        let displayed = update(&mut p, &progress);
        assert_eq!(displayed.state, refreshed.state);
        assert_eq!(displayed.current_path, refreshed.current_path);
        assert_eq!(displayed.summary, refreshed.summary);
        assert_eq!(displayed.detail, refreshed.detail);
    }

    cleanup.state = OperationState::Cancelled;
    cleanup.current_path = None;
    cleanup.completed = 2;
    cleanup.processed_entries = 3;
    let cancelled = update(&mut p, &cleanup);
    assert_eq!(cancelled.state, "Cancelled");
    assert_eq!(cancelled.progress_label, "3 items completed");
    assert!(cancelled.current_path.is_empty());
    assert_eq!(cancelled.activity_label, "Cancelled");
    assert!(!cancelled.active && !cancelled.discovering && !cancelled.waiting);
    assert!(!cancelled.paused && !cancelled.pause_requested && cancelled.eta.is_empty());
}

#[test]
#[ignore = "opt-in history projection timing; run alone with --ignored --nocapture"]
fn benchmark_warm_job_history_projection() {
    use std::{hint::black_box, time::Instant};
    const ITERATIONS: u32 = 8;
    for count in [1_000, 10_000] {
        for detailed in [false, true] {
            let mut p = Presenter::new("left".into(), "right".into());
            let samples: Arc<[ThroughputSample]> = (0..240)
                .map(|index| ThroughputSample {
                    start_millis: index * 250,
                    elapsed_millis: (index + 1) * 250,
                    transfer_per_second: 100_000.0 + index as f64,
                    verification_per_second: 80_000.0,
                    items_per_second: 0.0,
                })
                .collect();
            for index in 0..count {
                let mut value = progress(index as u64);
                value.state = CopyOperationState::Completed;
                value.current_path = None;
                value.completed_bytes = 1000;
                value.completed_directories = 3;
                value.copied_bytes = 1000;
                value.verified_bytes = 1000;
                value.discovery = OperationDiscovery {
                    state: DiscoveryState::Complete,
                    files: 2,
                    directories: 3,
                    bytes: Some(1000),
                    ..Default::default()
                };
                if detailed {
                    value.state = CopyOperationState::CompletedWithErrors;
                    value.failed = 16;
                    value.errors = (0..16)
                        .map(|error| {
                            FsError::from_io(
                                "copy files",
                                &PathBuf::from(format!("source/資料/{index}/entry-{error}")),
                                io::Error::other("retained failure detail ".repeat(12)),
                            )
                        })
                        .collect();
                    value.throughput = Arc::new(OperationThroughput {
                        samples: samples.clone(),
                        ..Default::default()
                    });
                }
                p.copies.progress.push(Arc::new(value));
            }
            // Populate chart caches before measuring unchanged history projection.
            assert_eq!(rows(&p).len(), count);
            for changing in [false, true] {
                let started = Instant::now();
                for tick in 0..ITERATIONS {
                    if changing {
                        let active = Arc::make_mut(p.copies.progress.last_mut().unwrap());
                        active.state = CopyOperationState::Running;
                        active.completed_bytes = 0;
                        active.completed_directories = 0;
                        active.files_completed = 0;
                        active.current_file_size = Some(1000);
                        active.current_file_copied = u64::from(tick) * 10;
                    }
                    drop(black_box(rows(black_box(&p))));
                }
                eprintln!(
                    "job-history rows={count} detailed={detailed} one_active={changing} \
                     iterations={ITERATIONS} mean_ms={:.3}",
                    started.elapsed().as_secs_f64() * 1000.0 / f64::from(ITERATIONS)
                );
            }
        }
    }
}

#[test]
fn queued_and_preparing_jobs_do_not_claim_active_discovery_or_calculation() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = CopyProgress::queued(OperationId(21), "target".into(), 1, TransferIntent::Copy);
    let queued = copy(&p, &value);
    assert_eq!(queued.progress_label, "Queued");
    assert_eq!(queued.activity_label, "Queued");
    assert!(queued.waiting && !queued.discovering);
    assert!(queued.eta.is_empty());
    value.state = CopyOperationState::Preparing;
    let preparing = copy(&p, &value);
    assert_eq!(preparing.progress_label, "Preparing");
    assert!(!preparing.discovering);
    assert_eq!(preparing.activity_label, "Preparing...");
    value.discovery.state = DiscoveryState::Scanning;
    let scanning = copy(&p, &value);
    assert!(scanning.discovering && !scanning.waiting);
    assert_eq!(scanning.activity_label, "Discovering items...");
}

#[test]
fn explicit_control_states_override_stale_scanning_in_presenter_rows() {
    let mut p = Presenter::new("left".into(), "right".into());
    let mut value = CopyProgress::queued(OperationId(22), "target".into(), 1, TransferIntent::Copy);
    value.discovery.state = DiscoveryState::Scanning;
    value.discovery.files = 27;
    for (state, paused, pause_requested, status) in [
        (
            CopyOperationState::CancelRequested,
            true,
            true,
            "Cancelling...",
        ),
        (
            CopyOperationState::WaitingForDecision,
            true,
            true,
            "Waiting for your decision",
        ),
        (CopyOperationState::Running, true, true, "Paused"),
        (CopyOperationState::Preparing, false, true, "Pausing..."),
        (CopyOperationState::Queued, false, false, "Queued"),
    ] {
        value.state = state;
        value.paused = paused;
        value.pause_requested = pause_requested;
        p.copies.progress = vec![Arc::new(value.clone())];
        let row = rows(&p).pop().expect("one captured transfer row");
        assert_eq!(row.activity_label, status, "activity for {state:?}");
        assert_eq!(row.progress_label, status, "headline for {state:?}");
        assert!(!row.discovering, "stale scanning masked {state:?}");
        assert!(
            row.eta.is_empty(),
            "inactive work acquired an ETA for {state:?}"
        );
    }
    value.state = CopyOperationState::Preparing;
    value.paused = false;
    value.pause_requested = false;
    p.copies.progress = vec![Arc::new(value)];
    let row = rows(&p).pop().unwrap();
    assert!(row.discovering);
    assert_eq!(row.activity_label, "Discovering items...");
    assert!(row.progress_label.starts_with("Discovering:"));
}

#[test]
fn typed_transfer_waits_override_stale_scanning_and_phase() {
    let p = Presenter::new("left".into(), "right".into());
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        for state in [
            CopyOperationState::Queued,
            CopyOperationState::Preparing,
            CopyOperationState::Running,
        ] {
            for (reason, label) in [
                (
                    TransferWaitReason::EarlierTransfers,
                    "Queued — waiting for earlier transfers",
                ),
                (
                    TransferWaitReason::Resources,
                    "Waiting for available capacity",
                ),
            ] {
                let mut value = CopyProgress::queued(OperationId(23), "target".into(), 1, intent);
                value.state = state;
                value.phase = CopyOperationPhase::Verifying;
                value.discovery.state = DiscoveryState::Scanning;
                value.discovery.files = 27;
                value.wait_reason = Some(reason);
                value.message = "Copying: this message is not an activity discriminator".into();
                let row = copy(&p, &value);
                assert_eq!(row.state, label);
                assert_eq!(row.activity_label, label);
                assert_eq!(row.progress_label, label);
                assert!(row.waiting && !row.discovering);
                assert!(row.eta.is_empty());
            }
        }
    }
}

#[test]
fn the_jobs_list_names_a_paused_transfer_with_its_phase() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = progress(26);
    for (phase, paused, pause_requested, state) in [
        (CopyOperationPhase::Copying, true, true, "Paused · copying"),
        (
            CopyOperationPhase::Verifying,
            true,
            true,
            "Paused · verifying SHA-256",
        ),
        (CopyOperationPhase::Copying, false, true, "Pausing..."),
        (CopyOperationPhase::Copying, true, false, "Resuming..."),
        (CopyOperationPhase::Copying, false, false, "Copying"),
    ] {
        value.phase = phase;
        value.paused = paused;
        value.pause_requested = pause_requested;
        let row = copy(&p, &value);
        assert_eq!(row.state, state);
        assert!(
            row.detail.contains(&format!("\n{state}\n")),
            "{}",
            row.detail
        );
    }
    // A decision, a cancellation and a finished job keep their own state.
    value.paused = true;
    value.pause_requested = true;
    for (state, shown) in [
        (CopyOperationState::WaitingForDecision, "Needs a decision"),
        (CopyOperationState::CancelRequested, "Cancelling"),
        (CopyOperationState::Completed, "Completed"),
    ] {
        value.state = state;
        assert_eq!(copy(&p, &value).state, shown);
    }
}

#[test]
fn control_states_and_pause_override_typed_waits_before_discovery() {
    let p = Presenter::new("left".into(), "right".into());
    for (state, paused, pause_requested, label) in [
        (
            CopyOperationState::CancelRequested,
            true,
            true,
            "Cancelling...",
        ),
        (
            CopyOperationState::WaitingForDecision,
            true,
            true,
            "Waiting for your decision",
        ),
        (CopyOperationState::Running, true, true, "Paused"),
        (CopyOperationState::Preparing, false, true, "Pausing..."),
        (CopyOperationState::Queued, true, true, "Paused"),
    ] {
        for reason in [
            TransferWaitReason::EarlierTransfers,
            TransferWaitReason::Resources,
        ] {
            let mut value =
                CopyProgress::queued(OperationId(24), "target".into(), 1, TransferIntent::Copy);
            value.state = state;
            value.paused = paused;
            value.pause_requested = pause_requested;
            value.wait_reason = Some(reason);
            value.discovery.state = DiscoveryState::Scanning;
            let row = copy(&p, &value);
            assert_eq!(row.activity_label, label);
            assert_eq!(row.progress_label, label);
            assert!(!row.waiting && !row.discovering);
            assert!(row.eta.is_empty());
        }
    }
}

#[test]
fn typed_wait_suppresses_eta_with_known_work_and_never_comes_from_message_text() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = CopyProgress::queued(OperationId(25), "target".into(), 1, TransferIntent::Copy);
    value.state = CopyOperationState::Running;
    value.phase = CopyOperationPhase::Copying;
    value.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        files: 1,
        bytes: Some(1000),
        ..Default::default()
    };
    value.current_file_size = Some(1000);
    value.current_file_copied = 500;
    value.remaining_work = Some(TransferRemainingWork {
        write_bytes: 500,
        verification_bytes: 1000,
    });
    value.throughput = Arc::new(OperationThroughput {
        smoothed_transfer_per_second: 100.0,
        ..Default::default()
    });
    value.wait_reason = Some(TransferWaitReason::Resources);
    let waiting = copy(&p, &value);
    assert!(waiting.waiting && !waiting.discovering);
    assert!(waiting.determinate && waiting.file_determinate);
    assert_eq!(waiting.progress, 0.5);
    assert_eq!(waiting.file_progress, 0.5);
    assert_eq!(waiting.progress_label, "Waiting for available capacity");
    assert!(waiting.eta.is_empty());

    value.wait_reason = None;
    value.message = "Waiting for available capacity".into();
    let active = copy(&p, &value);
    assert!(!active.waiting);
    assert_eq!(active.state, "Copying");
    assert_eq!(active.eta, "About 15 s");

    value.wait_reason = Some(TransferWaitReason::EarlierTransfers);
    value.state = CopyOperationState::Completed;
    value.paused = true;
    let completed = copy(&p, &value);
    assert_eq!(completed.state, "Completed");
    assert_eq!(completed.progress_label, "100% complete");
    assert!(completed.activity_label.is_empty());
    assert!(!completed.waiting && !completed.discovering);
    assert!(completed.eta.is_empty());
}

fn readback_history() -> Arc<OperationThroughput> {
    Arc::new(OperationThroughput {
        samples: vec![ThroughputSample {
            elapsed_millis: 250,
            verification_per_second: 1024.0,
            ..Default::default()
        }]
        .into(),
        verification_per_second: 1024.0,
        ..Default::default()
    })
}

/// A transfer's window charts the whole job in every phase; the current file's
/// readback shows in that file's own bar, not in a chart of its own.
#[test]
fn verifying_rows_chart_the_whole_job_and_the_files_bar_follows_its_readback() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = progress(26);
    value.current_file_size = Some(1000);
    value.current_file_copied = 1000;
    value.current_file_verified = 250;
    value.verification_pass_id = 1;
    value.verification_throughput = readback_history();
    value.throughput = Arc::new(OperationThroughput {
        samples: vec![ThroughputSample {
            elapsed_millis: 5000,
            transfer_per_second: 1_048_576.0,
            verification_per_second: 2_097_152.0,
            ..Default::default()
        }]
        .into(),
        transfer_per_second: 1_048_576.0,
        verification_per_second: 2_097_152.0,
        ..Default::default()
    });
    let verifying = copy(&p, &value);
    assert!(verifying.verification_view);
    assert_eq!(verifying.file_progress, 0.25);
    assert_eq!(verifying.file_percent, "Verifying 25%");
    assert_eq!(verifying.file_progress_label, "Verifying: 250 / 1000 bytes");
    assert_eq!(verifying.speed_chart, SpeedChartData::default());
    let chart = &verifying.transfer_speed_chart;
    assert_eq!(chart.speed, "1.0 MiB/s");
    assert_eq!(chart.verification_speed, "2.0 MiB/s");
    assert_eq!(chart.scale, "5 MiB/s");
    assert_eq!(chart.history, "Elapsed 5 s");

    value.wait_reason = Some(TransferWaitReason::Resources);
    let waiting = copy(&p, &value);
    assert!(waiting.waiting && !waiting.discovering);
    assert_eq!(waiting.file_progress, 0.25);
    assert_eq!(waiting.file_progress_label, "Verifying: 250 / 1000 bytes");
    assert_eq!(waiting.transfer_speed_chart, verifying.transfer_speed_chart);
    value.wait_reason = None;
    value.phase = CopyOperationPhase::ValidatingAfterMove;
    value.validation_bytes = 9000;
    let validation = copy(&p, &value);
    assert!(!validation.verification_view && !validation.file_determinate);
    assert_eq!(
        validation.transfer_speed_chart,
        verifying.transfer_speed_chart
    );
}

#[test]
fn verification_requires_a_nonzero_pass_and_current_file() {
    let p = Presenter::new("left".into(), "right".into());
    for (pass_id, size, eligible) in [
        (0, None, false),
        (0, Some(1000), false),
        (1, None, false),
        (1, Some(0), true),
        (1, Some(1000), true),
    ] {
        let mut value = progress(27);
        value.intent = TransferIntent::Move;
        value.native_entries_moved = 1;
        value.current_file_size = size;
        value.current_file_verified = 250;
        value.verification_pass_id = pass_id;
        value.verification_throughput = Arc::default();
        value.throughput = Arc::new(OperationThroughput {
            samples: vec![ThroughputSample {
                elapsed_millis: 250,
                items_per_second: 10.0,
                ..Default::default()
            }]
            .into(),
            items_per_second: 10.0,
            ..Default::default()
        });
        let row = copy(&p, &value);
        assert_eq!(
            row.verification_view, eligible,
            "pass {pass_id}, size {size:?}"
        );
        assert_eq!(row.speed_chart, SpeedChartData::default());
        if !eligible {
            assert!(!row.file_determinate);
            assert_eq!(row.file_progress, 0.0);
        }
        if pass_id == 0 {
            value.phase = CopyOperationPhase::CapturingSources;
            value.current_file_size = None;
            let renamed = copy(&p, &value);
            assert!(!renamed.verification_view && !renamed.file_determinate);
            assert!(renamed.transfer_speed_chart.native_move);
            assert_eq!(renamed.transfer_speed_chart.speed, "10.0 items/s");
        }
    }
}

#[test]
fn new_verification_pass_retains_resumed_checkpoint_bytes() {
    let mut p = Presenter::new("left".into(), "right".into());
    let mut value = progress(28);
    value.current_file_size = Some(1000);
    value.current_file_copied = 1000;
    value.current_file_verified = 250;
    value.verification_pass_id = 41;
    value.verification_throughput = readback_history();
    p.copies.progress = vec![Arc::new(value.clone())];
    let initial = rows(&p).pop().unwrap();
    assert_eq!(initial.file_progress, 0.25);

    value.current_file_verified = 500;
    p.copies.progress = vec![Arc::new(value.clone())];
    let running = rows(&p).pop().unwrap();
    assert_eq!(running.file_progress, 0.5);

    value.paused = true;
    value.pause_requested = true;
    p.copies.progress = vec![Arc::new(value.clone())];
    let paused = rows(&p).pop().unwrap();
    assert_eq!(paused.activity_label, "Paused");
    assert_eq!(paused.file_progress, 0.5);

    value.paused = false;
    value.pause_requested = false;
    value.verification_pass_id = 42;
    value.verification_throughput = Arc::default();
    p.copies.progress = vec![Arc::new(value.clone())];
    let retried = rows(&p).pop().unwrap();
    assert!(retried.verification_view && retried.file_determinate);
    assert_eq!(retried.file_progress, 0.5);
    assert_eq!(retried.file_progress_label, "Verifying: 500 / 1000 bytes");

    value.phase = CopyOperationPhase::Copying;
    value.verification_pass_id = 43;
    value.current_file_size = Some(2000);
    value.current_file_copied = 100;
    value.current_file_verified = 0;
    p.copies.progress = vec![Arc::new(value)];
    let next_file = rows(&p).pop().unwrap();
    assert!(!next_file.verification_view);
    assert_eq!(next_file.file_progress, 0.05);
    assert_eq!(
        next_file.file_progress_label,
        "Current file written: 100 / 2000 bytes"
    );
}

#[test]
fn terminal_verification_rows_keep_overall_history_after_the_attempt_is_cleared() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = progress(29);
    value.phase = CopyOperationPhase::Copying;
    value.throughput = Arc::new(OperationThroughput {
        samples: vec![ThroughputSample {
            elapsed_millis: 1000,
            transfer_per_second: 1_048_576.0,
            verification_per_second: 2048.0,
            ..Default::default()
        }]
        .into(),
        ..Default::default()
    });
    let original = copy(&p, &value).transfer_speed_chart;
    value.phase = CopyOperationPhase::Verifying;
    value.current_file_size = None;
    value.verification_pass_id = 0;
    value.verification_throughput = readback_history();
    for state in [
        CopyOperationState::Completed,
        CopyOperationState::CompletedWithErrors,
        CopyOperationState::Cancelled,
        CopyOperationState::Failed,
    ] {
        value.state = state;
        let terminal = copy(&p, &value);
        assert!(!terminal.verification_view && !terminal.file_determinate);
        assert_eq!(terminal.transfer_speed_chart, original);
        assert_eq!(terminal.transfer_speed_chart.speed, "0 B/s");
        assert_eq!(terminal.transfer_speed_chart.verification_speed, "0 B/s");
        assert_eq!(terminal.transfer_speed_chart.scale, "2 MiB/s");
        assert!(!terminal.transfer_speed_chart.primary.is_empty());
        assert_eq!(terminal.transfer_speed_chart.history, "Elapsed 1 s");
    }

    value.current_file_size = Some(1000);
    value.current_file_verified = 750;
    value.verification_pass_id = 9;
    let failed = copy(&p, &value);
    assert!(!failed.verification_view);
    assert_eq!(failed.transfer_speed_chart, original);
    assert!(failed.file_determinate);
    assert_eq!(failed.file_progress, 0.75);
    assert_eq!(failed.file_progress_label, "Verifying: 750 / 1000 bytes");
}

#[test]
fn accepted_local_copy_cancel_clears_pause_wait_without_retiring_or_rewriting_progress() {
    for (paused, pause_requested) in [(true, true), (false, true), (true, false)] {
        let mut p = Presenter::new("left".into(), "right".into());
        let mut value = progress(32);
        value.current_file_size = Some(1000);
        value.current_file_copied = 1000;
        value.current_file_verified = 750;
        value.copied_bytes = 1000;
        value.verified_bytes = 750;
        value.validation_bytes = 4000;
        value.verification_pass_id = 9;
        value.verification_throughput = readback_history();
        value.throughput = readback_history();
        value.paused = paused;
        value.pause_requested = pause_requested;
        value.wait_reason = Some(TransferWaitReason::Resources);
        let original = Arc::new(value);
        p.copies.progress = vec![original.clone()];
        p.cancel_copy("copy-job:32", &|_| false);
        assert!(
            Arc::ptr_eq(&p.copies.progress[0], &original),
            "refused cancellation must not normalize the live snapshot"
        );
        let commands = std::cell::RefCell::new(Vec::new());
        p.cancel_copy("copy-job:32", &|command| {
            commands.borrow_mut().push(command);
            true
        });
        assert!(matches!(
            commands.borrow().as_slice(),
            [Command::CancelCopy(OperationId(32))]
        ));
        let cancelling = p.copies.progress[0].clone();
        assert_eq!(cancelling.state, CopyOperationState::CancelRequested);
        assert!(!cancelling.paused && !cancelling.pause_requested);
        assert!(cancelling.wait_reason.is_none());
        assert_eq!(cancelling.current_file_copied, original.current_file_copied);
        assert_eq!(
            cancelling.current_file_verified,
            original.current_file_verified
        );
        assert_eq!(cancelling.copied_bytes, original.copied_bytes);
        assert_eq!(cancelling.verified_bytes, original.verified_bytes);
        assert_eq!(cancelling.validation_bytes, original.validation_bytes);
        assert_eq!(
            cancelling.verification_pass_id,
            original.verification_pass_id
        );
        assert!(Arc::ptr_eq(&cancelling.throughput, &original.throughput));
        assert!(Arc::ptr_eq(
            &cancelling.verification_throughput,
            &original.verification_throughput
        ));
        assert!(
            p.has_live_operations(),
            "accepted cancel is still awaiting actual teardown"
        );
        p.apply(Event::Copy(CopyEvent::Progress(original.clone())));
        assert!(
            Arc::ptr_eq(&p.copies.progress[0], &cancelling),
            "late nonterminal progress cannot restore stale pause state"
        );
        let row = rows(&p).pop().unwrap();
        assert_eq!(row.state, "Cancelling");
        assert_eq!(row.activity_label, "Cancelling...");
        assert!(!row.paused && !row.pause_requested && !row.waiting);
        assert_eq!(row.file_progress, 0.75);
        let mut terminal = (*original).clone();
        terminal.state = CopyOperationState::Cancelled;
        terminal.paused = false;
        terminal.pause_requested = false;
        terminal.wait_reason = None;
        terminal.current_file_size = None;
        terminal.verification_pass_id = 0;
        terminal.verification_throughput = Arc::default();
        p.apply(Event::Copy(CopyEvent::Progress(Arc::new(terminal))));
        assert!(!p.has_live_operations());
        assert_eq!(p.copies.progress[0].state, CopyOperationState::Cancelled);
    }
}

#[test]
fn each_transfer_wait_says_why_the_job_waits() {
    let p = Presenter::new("left".into(), "right".into());
    let cases = [
        (
            TransferWaitReason::DiskCapacity,
            "Waiting for disk capacity",
        ),
        (
            TransferWaitReason::SameFolders,
            "Waiting for another job in the same folders to finish",
        ),
        // Not "the same folders": a job that cannot confirm its folders runs alone.
        (
            TransferWaitReason::UnconfirmedFolders,
            "Waiting for another job: jobs whose folders cannot be confirmed run alone",
        ),
        (
            TransferWaitReason::OtherOperation,
            "Waiting for another file operation",
        ),
    ];
    for (reason, label) in cases {
        for state in [
            CopyOperationState::Queued,
            CopyOperationState::Preparing,
            CopyOperationState::Running,
        ] {
            let mut value =
                CopyProgress::queued(OperationId(26), "target".into(), 1, TransferIntent::Copy);
            value.state = state;
            value.wait_reason = Some(reason);
            let row = copy(&p, &value);
            assert_eq!(row.state, label, "{reason:?} in {state:?}");
            assert_eq!(row.activity_label, label);
            assert_eq!(row.progress_label, label);
            assert!(row.waiting && row.eta.is_empty());
            value.paused = true;
            value.pause_requested = true;
            assert_eq!(copy(&p, &value).activity_label, "Paused");
        }
    }
    assert_eq!(
        cases
            .iter()
            .map(|(_, label)| *label)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        cases.len(),
        "each reason has its own label"
    );
}

/// A running change of `kind`, as its job's first progress reports it.
fn running(id: u64, kind: MutationKind) -> Event {
    Event::Operation(OperationEvent::Progress(Arc::new(OperationProgress {
        id: OperationId(id),
        kind,
        state: OperationState::Running,
        pause_requested: false,
        paused: false,
        total: 2,
        completed: 0,
        failed: 0,
        processed_entries: 0,
        current_path: None,
        source_directory: "source".into(),
        renamed_to: None,
        message: String::new(),
        errors: vec![],
        throughput: Arc::default(),
        discovery: OperationDiscovery::default(),
    })))
}

/// The Delete job `id` cannot remove a locked folder and asks what to do.
fn delete_failure(id: u64, version: u64) -> Event {
    Event::Operation(OperationEvent::DeleteFailure(Arc::new(DeleteFailure {
        id: OperationId(id),
        version,
        mode: DeleteMode::Permanent,
        permanent_fallback: None,
        error: FsError::from_io(
            "mutate entry",
            std::path::Path::new("source/locked"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        ),
        can_retry: true,
        can_skip: true,
    })))
}

#[test]
fn a_waiting_delete_failure_marks_its_job_and_resolve_brings_it_up() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.apply(running(41, MutationKind::Delete(DeleteMode::Permanent)));
    let row = |p: &Presenter| {
        let mut jobs = rows(p);
        assert_eq!(jobs.len(), 1);
        jobs.pop().unwrap()
    };
    assert!(!row(&p).attention, "a running Delete needs nothing");
    p.apply(delete_failure(41, 1));
    let waiting = row(&p);
    assert!(waiting.attention, "the waiting question marks the row");
    assert_eq!(p.active_delete_decision(), Some((OperationId(41), 1)));
    // Decide later keeps the mark, and the row's Resolve shows the question again.
    p.defer_copy_decision("delete-error:41:1");
    assert_eq!(p.active_delete_decision(), None);
    assert!(row(&p).attention, "a deferred question still waits");
    p.resolve_copy_decision(&waiting.identity);
    assert_eq!(p.active_delete_decision(), Some((OperationId(41), 1)));
    // Answered, the job needs nothing more.
    p.decide_copy("delete-error:41:1", 0, String::new(), &|command| {
        matches!(command, Command::DecideDelete(_))
    });
    assert!(!row(&p).attention);
}

#[test]
fn resolve_brings_up_the_chosen_jobs_question_while_others_wait() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.apply(running(2, MutationKind::Rename));
    let item = |path: &str| MutationItem {
        path: path.into(),
        kind: EntryKind::File,
        metadata: EntryMetadata::default(),
    };
    p.apply(Event::Operation(OperationEvent::RenameConflict(Arc::new(
        CopyConflict {
            id: OperationId(2),
            version: 1,
            source: item("source/old.txt"),
            target: item("source/new.txt"),
            destination: "source/new.txt".into(),
            can_replace: true,
            can_merge: false,
        },
    ))));
    p.apply(running(41, MutationKind::Delete(DeleteMode::Permanent)));
    p.apply(delete_failure(41, 1));
    // The Rename question is shown first; the Delete one waits for its turn.
    assert_eq!(p.active_rename_decision(), Some((OperationId(2), 1)));
    assert_eq!(p.active_delete_decision(), None);
    let jobs = rows(&p);
    let delete = jobs.iter().find(|row| row.identity == "41").unwrap();
    assert!(delete.attention);
    // Resolve on the Delete job shows its question instead of the Rename one.
    p.resolve_copy_decision(&delete.identity);
    assert_eq!(
        p.active_rename_decision(),
        None,
        "the Rename question waits"
    );
    assert_eq!(p.active_delete_decision(), Some((OperationId(41), 1)));
    // Resolve on the Rename job brings it back, and on the Delete job again that one.
    p.resolve_copy_decision("rename-job:2");
    assert_eq!(p.active_rename_decision(), Some((OperationId(2), 1)));
    p.resolve_copy_decision("41");
    assert_eq!(p.active_rename_decision(), None);
    assert_eq!(p.active_delete_decision(), Some((OperationId(41), 1)));
    // Resolve on a waiting Copy job shows its question, not the Delete one chosen before.
    p.copies.progress.push(Arc::new(CopyProgress::queued(
        OperationId(9),
        "right".into(),
        1,
        TransferIntent::Copy,
    )));
    p.apply(Event::Copy(CopyEvent::Failure(Arc::new(CopyFailure {
        id: OperationId(9),
        version: 1,
        error: FsError::from_io(
            "write",
            std::path::Path::new("right/file"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        ),
        source: Some("source/file".into()),
        destination: "right/file".into(),
        stage: CopyErrorStage::Writing,
        can_retry: true,
        can_skip: true,
    }))));
    p.resolve_copy_decision("copy-job:9");
    assert_eq!(p.active_copy_decision(), Some((OperationId(9), 1)));
    assert_eq!(p.active_rename_decision(), None);
    assert_eq!(
        p.active_delete_decision(),
        None,
        "the Copy question is shown"
    );
    // Once answered, the questions that still wait come back in turn.
    p.decide_copy("copy-error:9:1", 0, String::new(), &|_| true);
    assert_eq!(p.active_copy_decision(), None);
    assert_eq!(p.active_rename_decision(), Some((OperationId(2), 1)));
}

/// A copy out of an archive, as the engine reports it while it extracts a member: the
/// progress window splits the member's path inside the archive file into its folder and
/// name, the counted totals make the bar determinate, and the estimate is shown.
#[test]
fn archive_extraction_shows_the_member_path_a_determinate_bar_and_an_estimate() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = progress(41);
    value.phase = CopyOperationPhase::Copying;
    let folder = PathBuf::from("data").join("backup.zip").join("docs");
    value.current_path = Some(folder.join("readme.txt"));
    value.current_destination = Some(PathBuf::from("target").join("docs").join("readme.txt"));
    value.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        scope: DiscoveryScope::Recursive,
        files: 4,
        directories: 1,
        bytes: Some(4000),
        ..Default::default()
    };
    value.files_completed = 2;
    value.completed_directories = 0;
    value.completed_bytes = 2000;
    value.copied_bytes = 2000;
    value.current_file_size = Some(1000);
    value.current_file_copied = 0;
    value.remaining_work = Some(TransferRemainingWork {
        write_bytes: 2000,
        verification_bytes: 2000,
    });
    value.throughput = Arc::new(OperationThroughput {
        smoothed_transfer_per_second: 100.0,
        smoothed_verification_per_second: 100.0,
        ..Default::default()
    });
    let row = copy(&p, &value);
    assert_eq!(row.transfer_directory, folder.to_string_lossy().as_ref());
    assert_eq!(row.transfer_name, "readme.txt");
    assert!(row.determinate, "{row:?}");
    assert_eq!(row.progress, 0.5);
    assert_eq!(
        row.transfer_totals,
        "Files: 2 of 4 · Bytes: 2.0 KiB of 3.9 KiB"
    );
    assert!(!row.eta.is_empty(), "{row:?}");
    assert!(
        row.detail.contains(&format!(
            "Current source\n{}",
            folder.join("readme.txt").display()
        )),
        "{}",
        row.detail
    );
}

#[test]
fn the_current_files_folder_and_name_lines_keep_the_folders_last_part_and_the_names_end() {
    let p = Presenter::new("left".into(), "right".into());
    let mut value = progress(71);
    let separator = std::path::MAIN_SEPARATOR;
    // An archive member: the archive's path, the folders inside it and the name.
    let folder = PathBuf::from("source")
        .join("backups")
        .join("2026.zip")
        .join("Raw footage")
        .join("Camera B");
    value.current_path = Some(folder.join("C0042 harbour sunrise wide shot.MP4"));
    let mut row = copy(&p, &value);
    current_file_tails(&mut row);
    assert_eq!(row.transfer_directory, folder.to_string_lossy().as_ref());
    assert_eq!(row.transfer_directory_tail, format!("{separator}Camera B"));
    assert_eq!(row.transfer_name, "C0042 harbour sunrise wide shot.MP4");
    assert_eq!(row.transfer_name_tail, "ide shot.MP4");
    assert!(row.transfer_directory.contains("2026.zip"));
    // A short name stays whole; a name of several scripts is cut between characters.
    assert_eq!(name_tail("a.txt"), "");
    assert_eq!(name_tail("twelve chars"), "");
    assert_eq!(
        name_tail("報告書の最終版のドラフト第三版.txt"),
        "のドラフト第三版.txt"
    );
    // Without a current file both lines are empty and keep their place.
    value.current_path = None;
    let mut row = copy(&p, &value);
    current_file_tails(&mut row);
    assert!(row.transfer_directory.is_empty() && row.transfer_name.is_empty());
    assert!(row.transfer_directory_tail.is_empty() && row.transfer_name_tail.is_empty());
}
