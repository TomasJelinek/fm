//! A folder total is final only after a clean completion of that folder's scan.
use super::*;

pub(super) fn file_size(bytes: u64, display: SizeDisplay, locale: &NumberLocale) -> String {
    match display {
        SizeDisplay::Readable => format_size(bytes, locale),
        SizeDisplay::Bytes => locale.format_u64(bytes),
    }
}

pub(super) fn folder_size(
    value: Option<&FolderSize>,
    running: bool,
    attempted: bool,
    display: SizeDisplay,
    locale: &NumberLocale,
) -> (String, i32, bool, String) {
    if let Some(value) = value {
        if value.finished && value.errors == 0 {
            return (
                file_size(value.bytes, display, locale),
                0,
                false,
                format!(
                    "Complete logical size; {} files. {} file links or directory cycles skipped; special files excluded.",
                    value.files, value.skipped_links
                ),
            );
        }
        let active = running && !value.finished;
        let state = if value.errors > 0 {
            3
        } else if active {
            1
        } else {
            2
        };
        let reason = if value.errors > 0 {
            let status = if value.finished {
                "Completed with warnings; this is a partial total."
            } else if active {
                "Calculating: some entries could not be counted."
            } else {
                "Unfinished or cancelled: partial total with warnings."
            };
            let details = value.warnings.as_ref().map_or_else(
                || format!("{} errors.", value.errors),
                |report| report.summary().tooltip.to_string(),
            );
            format!("{status}\n{details}")
        } else if active {
            "Calculating: the number is not final.".into()
        } else {
            "Unfinished or cancelled: this is a partial result.".into()
        };
        return (
            format!("≈ {}", file_size(value.bytes, display, locale)),
            state,
            active,
            reason,
        );
    }
    if running {
        (
            "…".into(),
            1,
            true,
            "Queued for calculation; no final size is available.".into(),
        )
    } else if attempted {
        (
            "—".into(),
            2,
            false,
            "Calculation did not finish for this folder.".into(),
        )
    } else {
        (
            "<DIR>".into(),
            0,
            false,
            "Folder size has not been calculated.".into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_cycle_omissions_are_explained_without_an_error_marker() {
        let size = FolderSize {
            bytes: 23,
            files: 3,
            skipped_links: 2,
            finished: true,
            ..Default::default()
        };
        let (text, state, running, tooltip) = folder_size(
            Some(&size),
            false,
            true,
            SizeDisplay::Bytes,
            &NumberLocale::default(),
        );
        assert_eq!((text.as_str(), state), ("23", 0));
        assert!(!running);
        assert!(tooltip.contains("2 file links or directory cycles skipped"));
    }

    #[test]
    fn actual_errors_are_distinct_from_unfinished_or_cancelled_estimates() {
        let locale = NumberLocale::default();
        for running in [false, true] {
            let failed = FolderSize {
                bytes: 4096,
                errors: 1,
                finished: !running,
                ..Default::default()
            };
            let (_, state, active, _) =
                folder_size(Some(&failed), running, true, SizeDisplay::Bytes, &locale);
            assert_eq!(
                state, 3,
                "an actual filesystem error needs its own indicator"
            );
            assert_eq!(active, running);
            let unfinished = FolderSize {
                bytes: 4096,
                ..Default::default()
            };
            let (_, state, active, _) = folder_size(
                Some(&unfinished),
                running,
                true,
                SizeDisplay::Bytes,
                &locale,
            );
            assert_ne!(state, 3, "unfinished data alone is not an error");
            assert_eq!(active, running);
        }
    }

    #[test]
    fn finished_warning_keeps_severity_without_progress_even_while_other_folders_run() {
        let locale = NumberLocale::default();
        for finished in [false, true] {
            for scan_running in [false, true] {
                let size = FolderSize {
                    bytes: 4096,
                    errors: 2,
                    finished,
                    ..Default::default()
                };
                let (text, state, active, tooltip) =
                    folder_size(Some(&size), scan_running, true, SizeDisplay::Bytes, &locale);
                assert_eq!(text, "≈ 4096");
                assert_eq!(state, 3);
                assert_eq!(active, scan_running && !finished);
                assert!(tooltip.contains("2 errors"));
                if finished {
                    assert!(tooltip.contains("Completed with warnings"));
                    assert!(!tooltip.contains("not final"));
                } else if scan_running {
                    assert!(tooltip.contains("Calculating"));
                } else {
                    assert!(tooltip.contains("Unfinished or cancelled"));
                }
            }
        }
    }

    #[test]
    fn completed_warning_keeps_cached_diagnostics_without_reading_the_report() {
        #[derive(Debug)]
        struct Report;
        impl SizeWarningSource for Report {
            fn snapshot(
                &self,
                _: &Cancellation,
            ) -> Result<std::sync::Arc<dyn GeneratedTextSource>, FsError> {
                panic!("presentation must not read the retained diagnostic report")
            }
        }
        let reason = "Access denied in child 資料; click for full details.";
        let size = FolderSize {
            bytes: 4096,
            errors: 1,
            finished: true,
            warnings: Some(SizeWarningReport::new(
                std::sync::Arc::new(Report),
                SizeWarningSummary {
                    observed: 1,
                    stored: 1,
                    tooltip: reason.into(),
                },
            )),
            ..Default::default()
        };
        let (text, state, active, tooltip) = folder_size(
            Some(&size),
            true,
            true,
            SizeDisplay::Bytes,
            &NumberLocale::default(),
        );
        assert_eq!(text, "≈ 4096");
        assert_eq!(state, 3);
        assert!(!active);
        assert!(tooltip.starts_with("Completed with warnings"));
        assert!(tooltip.ends_with(reason));
    }

    #[test]
    fn byte_mode_is_exact_even_above_floating_point_integer_precision() {
        assert_eq!(
            file_size(u64::MAX, SizeDisplay::Bytes, &NumberLocale::default()),
            "18446744073709551615"
        );
        assert_eq!(
            file_size(1024, SizeDisplay::Readable, &NumberLocale::default()),
            "1.0 KiB"
        );
        assert_eq!(
            file_size(1024, SizeDisplay::Bytes, &NumberLocale::default()),
            "1024"
        );
    }

    #[test]
    fn byte_mode_preserves_partial_folder_size_indication() {
        let partial = FolderSize {
            bytes: 123456789,
            ..Default::default()
        };
        let (text, state, running, _) = folder_size(
            Some(&partial),
            true,
            true,
            SizeDisplay::Bytes,
            &NumberLocale::default(),
        );
        assert_eq!(text, "≈ 123456789");
        assert_eq!(state, 1);
        assert!(running);
        let complete = FolderSize {
            finished: true,
            ..partial
        };
        let (text, state, running, _) = folder_size(
            Some(&complete),
            false,
            true,
            SizeDisplay::Bytes,
            &NumberLocale::default(),
        );
        assert_eq!(text, "123456789");
        assert_eq!(state, 0);
        assert!(!running);
    }

    #[test]
    fn locale_snapshot_controls_exact_grouping_readable_decimal_and_partial_sizes() {
        let locale = NumberLocale {
            decimal_separator: ",".into(),
            group_separator: "\u{a0}".into(),
            grouping: vec![3],
            repeat_last_group: true,
        };
        assert_eq!(
            file_size(u64::MAX, SizeDisplay::Bytes, &locale),
            "18\u{a0}446\u{a0}744\u{a0}073\u{a0}709\u{a0}551\u{a0}615"
        );
        assert_eq!(file_size(1536, SizeDisplay::Readable, &locale), "1,5 KiB");
        let partial = FolderSize {
            bytes: 12345,
            ..Default::default()
        };
        assert_eq!(
            folder_size(Some(&partial), true, true, SizeDisplay::Bytes, &locale).0,
            "≈ 12\u{a0}345"
        );
    }
}
