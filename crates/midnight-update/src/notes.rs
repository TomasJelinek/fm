//! Release notes as plain text: Markdown markup removed, cut to an excerpt.

/// The most characters of release notes shown; the release page has the rest.
pub const EXCERPT_CHARACTERS: usize = 1600;

/// Plain text for `markdown`: headings, emphasis, code marks and link targets go,
/// list items become bullets, and the text ends at a line break once it reaches
/// `EXCERPT_CHARACTERS`. Returns the text and whether it was cut.
pub fn excerpt(markdown: &str) -> (String, bool) {
    let mut lines: Vec<String> = Vec::new();
    let mut length = 0usize;
    let mut cut = false;
    let mut fenced = false;
    for raw in markdown.lines() {
        let trimmed = raw.trim_end();
        if trimmed.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        let line = if fenced {
            trimmed.to_owned()
        } else {
            plain_line(trimmed)
        };
        // One blank line at most between paragraphs, none at the start.
        if line.is_empty() && lines.last().is_none_or(String::is_empty) {
            continue;
        }
        let characters = line.chars().count() + 1;
        if length + characters > EXCERPT_CHARACTERS {
            cut = true;
            if lines.is_empty() {
                lines.push(line.chars().take(EXCERPT_CHARACTERS).collect());
            }
            break;
        }
        length += characters;
        lines.push(line);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    (lines.join("\n"), cut)
}

fn plain_line(line: &str) -> String {
    let indent = line.len() - line.trim_start().len();
    let mut text = line.trim_start();
    let mut prefix = String::new();
    if let Some(rest) = text.strip_prefix('#') {
        text = rest.trim_start_matches('#').trim_start();
    } else if let Some(rest) = text
        .strip_prefix("- ")
        .or_else(|| text.strip_prefix("* "))
        .or_else(|| text.strip_prefix("+ "))
    {
        prefix = format!("{}• ", " ".repeat(indent.min(8)));
        text = rest;
    } else if let Some(rest) = text.strip_prefix("> ") {
        text = rest;
    }
    if text.chars().all(|c| matches!(c, '-' | '*' | '_' | '=')) && text.len() >= 3 {
        return String::new();
    }
    prefix + &inline(text)
}

/// Removes `**`, `__`, backticks and link targets: `[text](url)` becomes `text`.
fn inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("**") || rest.starts_with("__") {
            rest = &rest[2..];
            continue;
        }
        if c == '`' {
            rest = &rest[1..];
            continue;
        }
        if c == '!' && rest[1..].starts_with('[') {
            rest = &rest[1..];
            continue;
        }
        if c == '['
            && let Some(close) = rest.find("](")
            && let Some(end) = rest[close + 2..].find(')')
        {
            out.push_str(&rest[1..close]);
            rest = &rest[close + 2 + end + 1..];
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}
