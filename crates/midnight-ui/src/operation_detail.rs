//! Bounded operation text; failure samples are independent of successful outcome samples.
use midnight_core::OperationProgress;
use std::fmt::Write;

pub(super) fn detail(progress: &OperationProgress, metadata_summary: &str) -> String {
    let mut text = format!(
        "{} / {} targets · {} errors · {} entries processed\n{}{}",
        progress.completed,
        progress.total,
        progress.failed,
        progress.processed_entries,
        progress
            .current_path
            .as_deref()
            .unwrap_or(&progress.source_directory)
            .display(),
        metadata_summary
    );
    if !progress.errors.is_empty() {
        text.push_str("\n\nReported issues:");
        // Outcomes and errors have separate backend budgets. A late failure must
        // remain visible even when the successful-outcome sample is already full.
        for error in progress.errors.iter().take(16) {
            let _ = write!(text, "\n{error}");
        }
    }
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
