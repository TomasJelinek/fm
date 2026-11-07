//! UI formatting only; native identities and the backend diagnostic formatter stay unchanged.
use midnight_core::FsError;

pub(crate) fn describe(error: &FsError) -> String {
    let mut text = error.message.clone();
    if !error.path.as_os_str().is_empty() {
        text.push_str("\n\nPath\n");
        text.push_str(&error.path.to_string_lossy());
    }
    text.push_str("\n\nOperation: ");
    text.push_str(error.operation);
    text
}

/// Backend samples are bounded; keep every retained failure readable as its own paragraph.
pub(crate) fn append_issues(text: &mut String, errors: &[FsError]) {
    if !errors.is_empty() {
        text.push_str("\n\nReported issues\n\n");
        text.push_str(&issues(errors, false));
    }
}

pub(crate) fn issues(errors: &[FsError], partial_changes: bool) -> String {
    let mut text = errors
        .iter()
        .take(16)
        .map(describe)
        .collect::<Vec<_>>()
        .join("\n\n");
    if errors.len() > 16 {
        text.push_str("\n\nMore issues are included in the job totals.");
    }
    if partial_changes {
        text.push_str("\n\nSome changes were already completed and have not been undone.");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io, path::Path};

    #[test]
    fn long_unicode_path_has_its_own_line_and_no_display_truncation() {
        let path = format!("資料/{}/žluťoučký 🦀.txt", "long name/".repeat(70));
        let error = FsError::from_io(
            "open file",
            Path::new(&path),
            io::Error::other("Access denied"),
        );
        assert_eq!(
            describe(&error),
            format!("Access denied\n\nPath\n{path}\n\nOperation: open file")
        );
        assert_eq!(error.path, Path::new(&path));
    }

    #[test]
    fn pathless_error_preserves_multiline_details_without_empty_path_label() {
        let error = FsError::from_io(
            "save options",
            Path::new(""),
            io::Error::other("Storage full\nRetry after freeing space."),
        );
        assert_eq!(
            describe(&error),
            "Storage full\nRetry after freeing space.\n\nOperation: save options"
        );
    }
}
