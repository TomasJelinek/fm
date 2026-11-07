//! Bounded operation text; failure samples are independent of successful outcome samples.
use midnight_core::OperationProgress;

pub(super) fn detail(progress: &OperationProgress, metadata_summary: &str) -> String {
    let mut text = format!(
        "{} / {} targets · {} errors · {} entries processed\n\nSource folder\n{}",
        progress.completed,
        progress.total,
        progress.failed,
        progress.processed_entries,
        progress.source_directory.display()
    );
    if let Some(path) = &progress.current_path {
        text.push_str(&format!("\n\nCurrent item\n{}", path.display()));
    }
    if !metadata_summary.is_empty() {
        text.push_str("\n\n");
        text.push_str(metadata_summary.trim());
    }
    // Failure samples remain independent of the successful-outcome sample.
    crate::error_presentation::append_issues(&mut text, &progress.errors);
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{FsError, MutationKind, OperationId, OperationState};
    use std::{io, path::PathBuf};

    #[test]
    fn late_failure_stays_visible_when_current_path_and_outcome_sample_have_moved_on() {
        let failed_path = PathBuf::from("parent/子目录/too-many-children");
        let progress = OperationProgress {
            id: OperationId(1),
            kind: MutationKind::EditMetadata,
            state: OperationState::CompletedWithErrors,
            total: 1,
            completed: 0,
            failed: 1,
            processed_entries: 70,
            current_path: Some(PathBuf::from("parent/last-successful-item")),
            message: String::new(),
            errors: vec![FsError::from_io(
                "snapshot metadata targets",
                &failed_path,
                io::Error::other("metadata directory snapshot exceeds its resource budget"),
            )],
            source_directory: PathBuf::from("parent"),
            renamed_to: None,
        };
        let text = detail(
            &progress,
            "\nAdditional outcomes are included in the totals.",
        );
        assert!(text.contains("last-successful-item"));
        assert!(text.contains("Additional outcomes"));
        assert!(text.contains(&failed_path.display().to_string()), "{text}");
        assert!(
            text.contains("snapshot exceeds its resource budget"),
            "{text}"
        );
    }
}
