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
) -> (String, i32, String) {
    if let Some(value) = value {
        if value.finished && value.errors == 0 {
            return (
                file_size(value.bytes, display, locale),
                0,
                format!(
                    "Complete logical size; {} files. Links and special files are excluded.",
                    value.files
                ),
            );
        }
        let state = if value.errors > 0 {
            3
        } else if running && !value.finished {
            1
        } else {
            2
        };
        let reason = if value.errors > 0 {
            format!(
                "partial result: {} errors; the number is not final.",
                value.errors
            )
        } else if running {
            "Calculating: the number is not final.".into()
        } else {
            "Unfinished or cancelled: this is a partial result.".into()
        };
        return (
            format!("≈ {}", file_size(value.bytes, display, locale)),
            state,
            reason,
        );
    }
    if running {
        (
            "…".into(),
            1,
            "Queued for calculation; no final size is available.".into(),
        )
    } else if attempted {
        (
            "—".into(),
            2,
            "Calculation did not finish for this folder.".into(),
        )
    } else {
        (
            "<DIR>".into(),
            0,
            "Folder size has not been calculated.".into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let (_, state, _) =
                folder_size(Some(&failed), running, true, SizeDisplay::Bytes, &locale);
            assert_eq!(
                state, 3,
                "an actual filesystem error needs its own indicator"
            );
            let unfinished = FolderSize {
                bytes: 4096,
                ..Default::default()
            };
            let (_, state, _) = folder_size(
                Some(&unfinished),
                running,
                true,
                SizeDisplay::Bytes,
                &locale,
            );
            assert_ne!(state, 3, "unfinished data alone is not an error");
        }
    }

    #[test]
    fn byte_mode_is_exact_even_above_floating_point_integer_precision() {
        assert_eq!(
            file_size(u64::MAX, SizeDisplay::Bytes, &NumberLocale::default()),
            "18446744073709551615"
        );
        assert_eq!(
            file_size(1024, SizeDisplay::Readable, &NumberLocale::default()),
            "1.0 KB"
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
        let (text, state, _) = folder_size(
            Some(&partial),
            true,
            true,
            SizeDisplay::Bytes,
            &NumberLocale::default(),
        );
        assert_eq!(text, "≈ 123456789");
        assert_eq!(state, 1);
        let complete = FolderSize {
            finished: true,
            ..partial
        };
        let (text, state, _) = folder_size(
            Some(&complete),
            false,
            true,
            SizeDisplay::Bytes,
            &NumberLocale::default(),
        );
        assert_eq!(text, "123456789");
        assert_eq!(state, 0);
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
        assert_eq!(file_size(1536, SizeDisplay::Readable, &locale), "1,5 KB");
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
