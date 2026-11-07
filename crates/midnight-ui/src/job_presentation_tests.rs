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
    assert!(!row.determinate);
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
fn copy_details_show_retained_originals_and_truthful_bounded_samples() {
    let p = Presenter::new("left".into(), "right".into());
    let mut progress = progress(3);
    progress.state = CopyOperationState::Completed;
    progress.recovery_count = 20;
    progress.recovery_paths = vec![PathBuf::from("destination/復元/original")];
    let row = copy(&p, &progress);
    assert!(row.detail.contains("20 recovery files retained"));
    assert!(row.detail.contains("destination/復元/original"));
    assert!(row.detail.contains("Additional recovery files"));
    assert!(!row.failed);
}

#[test]
fn copy_and_mutation_jobs_share_chronological_order_and_stable_identity() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.copies.progress = vec![Arc::new(progress(1)), Arc::new(progress(3))];
    p.operations.progress.push(Arc::new(OperationProgress {
        id: OperationId(2),
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
        ["copy-job:3", "2", "copy-job:1"]
    );
    assert!(rows[1].determinate);
    assert_eq!(rows[1].progress, 0.0);
}

#[test]
fn recoverable_move_reports_original_locations_and_never_claims_source_space_was_freed() {
    let p = Presenter::new("left".into(), "right".into());
    let mut progress = progress(4);
    progress.intent = TransferIntent::RecoverableMove;
    progress.phase = CopyOperationPhase::ValidatingAfterMove;
    progress.source_names_captured = 1;
    progress.source_recovery_count = 1;
    progress.validation_bytes = 246;
    progress.source_recoveries.push(MoveSourceRecovery {
        source: "source/Žluťoučký.txt".into(),
        recovery: TransferRecovery {
            path: "source/.midnight-recovery-test/Žluťoučký.txt".into(),
            identity: None,
        },
        matched_source: true,
    });
    let row = copy(&p, &progress);
    assert_eq!(row.title, "Move (originals retained)");
    assert_eq!(row.state, "Checking retained originals");
    assert!(row.detail.contains("1 source names relocated"));
    assert!(row.detail.contains("Source space is not freed."));
    assert!(row.detail.contains("Original source\nsource/Žluťoučký.txt"));
    assert!(
        row.detail
            .contains("Recovery\nsource/.midnight-recovery-test/Žluťoučký.txt")
    );
    assert!(!row.failed);
    assert!(!row.determinate);
}
