use super::*;
use std::{io, path::PathBuf, sync::Arc};

fn progress(id: u64) -> CopyProgress {
    CopyProgress {
        id: OperationId(id),
        state: CopyOperationState::Running,
        phase: CopyOperationPhase::Verifying,
        requested_roots: 1,
        files_completed: 2,
        directories_created: 3,
        skipped: 0,
        failed: 0,
        copied_bytes: u64::MAX,
        verified_bytes: 123,
        current_path: Some(
            PathBuf::from("資料")
                .join("long/".repeat(100))
                .join("🦀.txt"),
        ),
        destination_directory: "destination/資料".into(),
        message: String::new(),
        errors: vec![],
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
