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

/// Compact presentation only: retain the original diagnostic separately and never decode escapes.
pub(crate) fn folder_reason(message: &str, path: &Path) -> String {
    let (reason, _, _) = folder_context(message, path);
    let reason = reason.lines().next().unwrap_or_default().trim();
    if reason.is_empty() {
        "The folder could not be read.".into()
    } else {
        reason.into()
    }
}

/// Show original bounded diagnostic lines, not the report log's escaped record encoding.
pub(crate) fn folder_details(message: &str, path: &Path) -> String {
    let (reason, operation, native) = folder_context(message, path);
    let mut lines: Vec<String> = reason.lines().skip(1).map(str::to_owned).collect();
    if let Some(operation) = operation {
        lines.push(format!("Operation: {operation}"));
    }
    if let Some(native) = native {
        // The first line is the debug-formatted native path, already shown separately.
        lines.extend(native.lines().skip(1).map(str::to_owned));
    }
    lines.join("\n").trim().to_owned()
}

fn folder_context<'a>(
    message: &'a str,
    path: &Path,
) -> (&'a str, Option<&'a str>, Option<&'a str>) {
    let (context, native) = message
        .split_once("\nNative path: ")
        .map_or((message, None), |(context, native)| (context, Some(native)));
    let suffix = format!(" ({})", path.display());
    let (reason, operation) = context
        .strip_suffix(&suffix)
        .and_then(|context| context.split_once(": "))
        .map_or((context, None), |(operation, reason)| {
            (reason, Some(operation))
        });
    (reason, operation, native)
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
    fn folder_reason_removes_repeated_native_context_without_decoding_path_escapes() {
        let path = Path::new(r"C:\new\names");
        let message = format!(
            "read_directory: Access denied (os error 5) ({})\nNative path: {:?}\nError kind: PermissionDenied",
            path.display(),
            path,
        );
        assert_eq!(folder_reason(&message, path), "Access denied (os error 5)");
        let mut display = String::new();
        append_path(&mut display, path);
        assert_eq!(display, r"C:\new\names");
        assert!(message.contains("Native path:"));
        assert_eq!(
            folder_details(&message, path),
            "Operation: read_directory\nError kind: PermissionDenied"
        );
        assert!(!folder_details(&message, path).contains(r"\n"));
    }

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
