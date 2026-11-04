//! Insert literal paths only where a shell argument can safely begin. This is a
//! conservative editor check, not an interpreter for user-submitted shell code.
use fm_core::ShellFlavor;

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

/// Slint reports UTF-8 byte offsets. Clamp invalid offsets without normalizing
/// combining marks, emoji or the command's unchanged prefix/suffix.
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

fn single_quote(ch: char, shell: ShellFlavor) -> bool {
    ch == '\'' || (shell == ShellFlavor::PowerShell && matches!(ch, '\u{2018}' | '\u{2019}'))
}

/// Deliberately recognizes plain arguments and balanced quoted arguments only.
/// Unsupported shell syntax remains executable on explicit submission, but the
/// path shortcut refuses to modify it: an inner single quote does not protect a
/// filename's `$()` when inserted into an already double-quoted argument.
pub(crate) fn safe_path_context(text: &str, anchor: i32, cursor: i32, shell: ShellFlavor) -> bool {
    let (start, end) = selection_bounds(text, anchor, cursor);
    if start == 0 && end == text.len() {
        return true;
    }
    let separator = |ch: char| ch == ' ' || ch == '\t';
    if !text[..start].chars().next_back().is_none_or(separator)
        || !text[end..].chars().next().is_none_or(separator)
        || text.contains(['\n', '\r', '\0'])
    {
        return false;
    }
    if shell == ShellFlavor::Cmd {
        return safe_cmd_context(text, start, end);
    }
    let mut quote = Quote::None;
    let mut start_quote = None;
    let mut end_quote = None;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if index == start {
            start_quote = Some(quote);
        }
        if index == end {
            end_quote = Some(quote);
        }
        match quote {
            Quote::Single => {
                if single_quote(ch, shell) {
                    if shell == ShellFlavor::PowerShell
                        && chars.peek().is_some_and(|(_, next)| *next == ch)
                    {
                        chars.next();
                    } else {
                        quote = Quote::None;
                    }
                }
            }
            Quote::Double => {
                if ch == '"' {
                    quote = Quote::None;
                } else if matches!(ch, '$' | '`' | '\u{201c}' | '\u{201d}')
                    || (ch == '\\' && shell == ShellFlavor::Posix)
                {
                    return false;
                }
            }
            Quote::None => {
                if single_quote(ch, shell) {
                    quote = Quote::Single;
                } else if ch == '"' {
                    quote = Quote::Double;
                } else if ch == '\\' && shell == ShellFlavor::Posix {
                    // This is the sole unquoted escape produced by our POSIX
                    // path serializer: 'one'\''two'. Escaped whitespace and
                    // arbitrary escapes would invalidate boundary checks.
                    if !chars.next().is_some_and(|(_, next)| next == '\'') {
                        return false;
                    }
                } else if matches!(
                    ch,
                    '$' | '`'
                        | '#'
                        | ';'
                        | '&'
                        | '|'
                        | '<'
                        | '>'
                        | '('
                        | ')'
                        | '{'
                        | '}'
                        | '\u{201c}'
                        | '\u{201d}'
                ) || (shell == ShellFlavor::PowerShell
                    && (ch == '@' || text[index..].starts_with("--%")))
                {
                    return false;
                }
            }
        }
    }
    if start == text.len() {
        start_quote = Some(quote);
    }
    if end == text.len() {
        end_quote = Some(quote);
    }
    quote == Quote::None && start_quote == Some(Quote::None) && end_quote == Some(Quote::None)
}

fn safe_cmd_context(text: &str, start: usize, end: usize) -> bool {
    let mut quoted = false;
    let mut start_quoted = None;
    let mut end_quoted = None;
    for (index, ch) in text.char_indices() {
        if index == start {
            start_quoted = Some(quoted);
        }
        if index == end {
            end_quoted = Some(quoted);
        }
        // Percent expansion occurs inside quotes; caret escaping and command
        // operators make the insertion context ambiguous. /V:OFF makes ! literal.
        if matches!(ch, '%' | '^') {
            return false;
        }
        if ch == '"' {
            quoted = !quoted;
        } else if !quoted && matches!(ch, '&' | '|' | '<' | '>' | '(' | ')' | '\'' | '`') {
            return false;
        }
    }
    if start == text.len() {
        start_quoted = Some(quoted);
    }
    if end == text.len() {
        end_quoted = Some(quoted);
    }
    !quoted && start_quoted == Some(false) && end_quoted == Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_insertion_rejects_expansion_escape_and_nonliteral_contexts() {
        assert!(safe_path_context("echo  ", 5, 5, ShellFlavor::Cmd));
        let quoted = "copy \"C:\\文档's & !\" ";
        assert!(safe_path_context(
            quoted,
            quoted.len() as i32,
            quoted.len() as i32,
            ShellFlavor::Cmd
        ));
        for text in [
            "echo %PATH% ",
            "echo \"%PATH%\" ",
            "echo a^ b ",
            "echo x & other ",
            "echo (x) ",
            "echo 'prior shell quotes' ",
        ] {
            assert!(
                !safe_path_context(text, text.len() as i32, text.len() as i32, ShellFlavor::Cmd),
                "cmd context must be rejected: {text:?}"
            );
        }
        assert!(!safe_path_context("echo \"\"", 6, 6, ShellFlavor::Cmd));
    }

    #[test]
    fn insertion_requires_a_standalone_argument_outside_existing_quotes() {
        for shell in [ShellFlavor::Posix, ShellFlavor::PowerShell] {
            assert!(safe_path_context("cp  ", 3, 3, shell));
            assert!(safe_path_context("echo OLD tail", 5, 8, shell));
            assert!(safe_path_context("echo \"old path\" tail", 5, 15, shell));
            assert!(!safe_path_context("printf \"%s\" \"\"", 13, 13, shell));
            assert!(!safe_path_context("echo ''", 6, 6, shell));
            assert!(!safe_path_context("echo prefixsuffix", 11, 11, shell));
            for text in [
                "echo $(run) ",
                "echo `run` ",
                "echo # comment ",
                "echo x | cat ",
                "cat << EOF ",
                "echo \"unfinished ",
                "echo 'unfinished ",
                "echo x\n ",
            ] {
                assert!(
                    !safe_path_context(text, text.len() as i32, text.len() as i32, shell),
                    "unexpected accepted context: {text:?}"
                );
            }
            assert!(safe_path_context("$(discard all)", 0, 14, shell));
        }
        assert!(!safe_path_context(
            "native --% ",
            11,
            11,
            ShellFlavor::PowerShell
        ));
        assert!(!safe_path_context(
            "echo @'here' ",
            13,
            13,
            ShellFlavor::PowerShell
        ));
    }

    #[test]
    fn a_second_path_can_follow_generated_quoted_unicode_and_apostrophes() {
        let posix = "cp '文档'\\''s $(literal)' ";
        assert!(safe_path_context(
            posix,
            posix.len() as i32,
            posix.len() as i32,
            ShellFlavor::Posix
        ));
        let powershell = "cp '文档''s ‘‘ ’’ $(literal)' ";
        assert!(safe_path_context(
            powershell,
            powershell.len() as i32,
            powershell.len() as i32,
            ShellFlavor::PowerShell
        ));
    }
}
