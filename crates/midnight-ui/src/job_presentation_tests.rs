use super::*;
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
            .contains("1 temporary source locations need attention")
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
            .contains("1 temporary source locations need attention")
    );
    assert!(row.detail.contains("2 source removals accepted by Windows"));
    assert!(
        row.detail
            .contains("1 source namespace changes without a confirmed")
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
    value.current_file_size = Some(1_000);
    value.current_file_copied = 500;
    let row = copy(&p, &value);
    assert_eq!(row.progress, 0.25);
    assert_eq!(row.file_progress, 0.5);
    assert!(row.file_determinate && row.active && !row.completed);
    assert_eq!(row.progress_label, "25% complete");
    value.current_file_copied = 1_000;
    value.phase = CopyOperationPhase::Verifying;
    let row = copy(&p, &value);
    assert_eq!(row.file_progress, 1.0);
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
