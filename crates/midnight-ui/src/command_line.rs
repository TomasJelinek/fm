//! Raw command-editor insertion. Editing never starts a terminal or interprets shell syntax.
use std::{borrow::Cow, path::Path};

fn selection_bounds(text: &str, anchor: i32, cursor: i32) -> (usize, usize) {
    let boundary = |offset: i32| {
        let mut offset = usize::try_from(offset).unwrap_or(0).min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    };
    let (anchor, cursor) = (boundary(anchor), boundary(cursor));
    (anchor.min(cursor), anchor.max(cursor))
}

/// Slint reports UTF-8 byte offsets. Preserve the unchanged prefix/suffix and
/// every character in the inserted value, including its actual spaces and quotes.
pub(crate) fn replace_command_selection(
    text: &str,
    anchor: i32,
    cursor: i32,
    token: &str,
) -> (String, i32) {
    let (start, end) = selection_bounds(text, anchor, cursor);
    let mut result = String::with_capacity(text.len() - (end - start) + token.len());
    result.push_str(&text[..start]);
    result.push_str(token);
    result.push_str(&text[end..]);
    (
        result,
        i32::try_from(start + token.len()).unwrap_or(i32::MAX),
    )
}

/// Do not turn a lossy display caption into a different native filename, or
/// silently insert a line break/control character into the single-line editor.
pub(crate) fn command_path_text(path: &Path) -> Result<&str, &'static str> {
    let text = path
        .to_str()
        .ok_or("This name cannot be represented exactly in the command-line editor.")?;
    if text.chars().any(char::is_control) {
        return Err(
            "This name contains control characters that cannot be inserted into a single command line.",
        );
    }
    Ok(text)
}

/// A directory insertion leaves the caret ready for a filename. Preserve native
/// spelling, including an existing separator, without querying the filesystem.
pub(crate) fn command_directory_text(path: &Path) -> Result<Cow<'_, str>, &'static str> {
    let text = command_path_text(path)?;
    if text.is_empty() || text.ends_with(std::path::is_separator) {
        Ok(Cow::Borrowed(text))
    } else {
        let mut directory = text.to_owned();
        directory.push(std::path::MAIN_SEPARATOR);
        Ok(Cow::Owned(directory))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insertion_preserves_unicode_spaces_and_shell_characters_without_wrapping() {
        let name = " Žlutý ' $(data) & 文档 👩🏽‍💻.txt ";
        assert_eq!(command_path_text(Path::new(name)).unwrap(), name);
        let (text, caret) = replace_command_selection("prefix", 6, 6, name);
        assert_eq!(text, format!("prefix{name}"));
        assert_eq!(caret as usize, text.len());
    }

    #[test]
    fn directory_insertion_ends_with_a_native_separator_without_shell_formatting() {
        for name in ["my folder", " Žlutý ' $(data) & 文档 👩🏽‍💻 "] {
            assert_eq!(
                command_directory_text(Path::new(name)).unwrap(),
                format!("{name}{}", std::path::MAIN_SEPARATOR)
            );
        }
        let long = "文档".repeat(300);
        assert_eq!(
            command_directory_text(Path::new(&long)).unwrap(),
            format!("{long}{}", std::path::MAIN_SEPARATOR)
        );
    }

    #[test]
    fn directory_insertion_preserves_existing_separators_roots_and_empty_paths() {
        for path in [
            String::new(),
            std::path::MAIN_SEPARATOR_STR.into(),
            format!("folder{}", std::path::MAIN_SEPARATOR),
            "folder/".into(),
        ] {
            assert_eq!(command_directory_text(Path::new(&path)).unwrap(), path);
        }
    }

    #[test]
    fn caller_can_insert_inside_an_existing_quote_without_inventing_shell_syntax() {
        let (text, caret) = replace_command_selection("echo \"\" tail", 6, 6, "my file");
        assert_eq!(text, "echo \"my file\" tail");
        assert_eq!(caret, 13);
        assert_eq!(replace_command_selection("abc", 0, 3, "x"), ("x".into(), 1));
    }

    #[test]
    fn selection_clamps_to_unicode_boundaries_without_normalization() {
        assert_eq!(
            replace_command_selection("文档 tail", 1, 5, "x"),
            ("x档 tail".into(), 1)
        );
        assert_eq!(
            replace_command_selection("hi", -2, 90, "x"),
            ("x".into(), 1)
        );
    }

    #[test]
    fn single_line_editor_rejects_control_characters_instead_of_changing_the_name() {
        for name in [
            "one\ntwo",
            "one\rtwo",
            "one\0two",
            "one\ttwo",
            "one\u{1b}two",
        ] {
            assert!(command_path_text(Path::new(name)).is_err());
            assert!(command_directory_text(Path::new(name)).is_err());
        }
    }
}
