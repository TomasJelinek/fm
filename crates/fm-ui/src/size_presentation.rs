//! A folder total is final only after a clean completion of that folder's scan.
use super::*;

pub(super) fn folder_size(
    value: Option<&FolderSize>,
    running: bool,
    attempted: bool,
) -> (String, i32, String) {
    if let Some(value) = value {
        if value.finished && value.errors == 0 {
            return (
                format_size(value.bytes),
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
        return (format!("≈ {}", format_size(value.bytes)), state, reason);
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
