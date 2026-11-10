//! UI formatting only; native identities and the backend diagnostic formatter stay unchanged.
use midnight_core::FsError;
use std::{fmt::Write, path::Path};

/// Keep ordinary Unicode readable and escape native units or control characters
/// that would otherwise lose identity or inject extra diagnostic fields.
pub(crate) fn append_path(text: &mut String, path: &Path) {
    match path.to_str() {
        Some(path) if !path.chars().any(char::is_control) => text.push_str(path),
        _ => {
            let _ = write!(text, "{path:?}");
        }
    }
}

pub(crate) fn describe(error: &FsError) -> String {
    let mut text = error.message.clone();
    if !error.path.as_os_str().is_empty() {
        text.push_str("\n\nPath\n");
        append_path(&mut text, &error.path);
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

    #[test]
    fn control_characters_in_a_path_cannot_forge_error_fields_or_lines() {
        let path = Path::new("資料/a\n\nOperation: forged\t\u{1b}[31m.txt");
        let error = FsError::from_io("open file", path, io::Error::other("Access denied"));
        let details = describe(&error);
        assert!(details.contains("資料/a\\n\\nOperation: forged\\t"));
        assert!(!details.contains('\u{1b}'));
        assert!(!details.contains('\t'));
        assert_eq!(details.matches("\n\nOperation: ").count(), 1);
        assert!(details.ends_with("\n\nOperation: open file"));
        assert_eq!(error.path, path);
    }
}

#[cfg(all(test, target_os = "windows"))]
#[path = "error_presentation_windows_tests.rs"]
mod windows_tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "error_presentation_linux_tests.rs"]
mod linux_tests;
