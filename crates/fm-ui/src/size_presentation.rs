//! A folder total is final only after a clean completion of that folder's scan.
use super::*;

pub(super) fn file_size(bytes: u64, display: SizeDisplay) -> String {
    match display {
        SizeDisplay::Readable => format_size(bytes),
        SizeDisplay::Bytes => bytes.to_string(),
    }
}

pub(super) fn folder_size(
    value: Option<&FolderSize>,
    running: bool,
    attempted: bool,
    display: SizeDisplay,
) -> (String, i32, String) {
    if let Some(value) = value {
        if value.finished && value.errors == 0 {
            return (
                file_size(value.bytes, display),
                0,
                format!(
                    "Complete logical size; {} files. Links and special files are excluded.",
                    value.files
                ),
            );
        }
        let state = if running && !value.finished { 1 } else { 2 };
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
            format!("≈ {}", file_size(value.bytes, display)),
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
    fn byte_mode_is_exact_even_above_floating_point_integer_precision() {
        assert_eq!(
            file_size(u64::MAX, SizeDisplay::Bytes),
            "18446744073709551615"
        );
        assert_eq!(file_size(1024, SizeDisplay::Readable), "1.0 KB");
        assert_eq!(file_size(1024, SizeDisplay::Bytes), "1024");
    }

    #[test]
    fn byte_mode_preserves_partial_folder_size_indication() {
        let partial = FolderSize {
            bytes: 123456789,
            ..Default::default()
        };
        let (text, state, _) = folder_size(Some(&partial), true, true, SizeDisplay::Bytes);
        assert_eq!(text, "≈ 123456789");
        assert_eq!(state, 1);
        let complete = FolderSize {
            finished: true,
            ..partial
        };
        let (text, state, _) = folder_size(Some(&complete), false, true, SizeDisplay::Bytes);
        assert_eq!(text, "123456789");
        assert_eq!(state, 0);
    }
}
