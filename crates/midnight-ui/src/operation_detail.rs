//! Bounded operation text; failure samples are independent of successful outcome samples.
use crate::{error_presentation::append_path, job_presentation::counted};
use midnight_core::{NumberLocale, OperationProgress};

pub(super) fn detail(
    progress: &OperationProgress,
    metadata_summary: &str,
    locale: &NumberLocale,
) -> String {
    // Counts follow regional grouping, as in the job's summary line; paths keep
    // their identity and cannot inject extra lines (see `append_path`).
    let mut text = format!(
        "{} / {} · {} · {} processed\n\nSource folder\n",
        locale.format_u64(progress.completed),
        counted(progress.total, "target", "targets", locale),
        counted(progress.failed, "error", "errors", locale),
        counted(progress.processed_entries, "entry", "entries", locale),
    );
    append_path(&mut text, &progress.source_directory);
    if let Some(path) = &progress.current_path {
        text.push_str("\n\nCurrent item\n");
        append_path(&mut text, path);
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

    fn progress() -> OperationProgress {
        OperationProgress {
            pause_requested: false,
            paused: false,
            discovery: Default::default(),
            id: OperationId(1),
            throughput: Default::default(),
            kind: MutationKind::EditMetadata,
            state: OperationState::CompletedWithErrors,
            total: 1,
            completed: 0,
            failed: 1,
            processed_entries: 70,
            current_path: Some(PathBuf::from("parent/last-successful-item")),
            message: String::new(),
            errors: Vec::new(),
            source_directory: PathBuf::from("parent"),
            renamed_to: None,
        }
    }

    #[test]
    fn late_failure_stays_visible_when_current_path_and_outcome_sample_have_moved_on() {
        let failed_path = PathBuf::from("parent/子目录/too-many-children");
        let progress = OperationProgress {
            errors: vec![FsError::from_io(
                "snapshot metadata targets",
                &failed_path,
                io::Error::other("metadata directory snapshot exceeds its resource budget"),
            )],
            ..progress()
        };
        let text = detail(
            &progress,
            "\nAdditional outcomes are included in the totals.",
            &NumberLocale::default(),
        );
        assert!(text.contains("last-successful-item"));
        assert!(text.contains("Additional outcomes"));
        assert!(text.contains(&failed_path.display().to_string()), "{text}");
        assert!(
            text.contains("snapshot exceeds its resource budget"),
            "{text}"
        );
    }

    #[test]
    fn counts_follow_regional_grouping_and_paths_cannot_add_lines() {
        let locale = NumberLocale {
            decimal_separator: ",".into(),
            group_separator: ".".into(),
            grouping: vec![3],
            repeat_last_group: true,
        };
        let progress = OperationProgress {
            total: 20_000,
            completed: 12_345,
            failed: 1_000,
            processed_entries: 1_234_567,
            current_path: Some(PathBuf::from("parent/a\n\nCurrent item\nforged")),
            ..progress()
        };
        let text = detail(&progress, "", &locale);
        assert!(
            text.starts_with(
                "12.345 / 20.000 targets · 1.000 errors · 1.234.567 entries processed"
            ),
            "{text}"
        );
        assert_eq!(
            text.lines().filter(|line| *line == "Current item").count(),
            1,
            "{text}"
        );
        assert!(!text.contains("\nforged"), "{text}");
    }

    #[test]
    fn a_count_of_one_is_singular() {
        let progress = OperationProgress {
            total: 1,
            completed: 1,
            failed: 1,
            processed_entries: 1,
            ..progress()
        };
        let text = detail(&progress, "", &NumberLocale::default());
        assert!(
            text.starts_with("1 / 1 target · 1 error · 1 entry processed\n"),
            "{text}"
        );
    }
}
