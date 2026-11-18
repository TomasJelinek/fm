//! Resynchronization after a top-level block longer than a read.
//!
//! A parse can prove boundaries only inside its own read, so such a block
//! would otherwise end checkpoint discovery for the rest of the document. Three
//! kinds of top-level block have a purely lexical end, which plain reads that
//! continue the scan contiguously find, proving the boundary after it:
//!
//! - A fenced code block ends at the first later closing fence of its marker.
//! - An HTML block of CommonMark type 6 or 7 ends at its first blank line.
//! - A GFM table ends at its first blank line. A table also ends at a line that
//!   starts another block, and that block can contain blank lines (a fence, or
//!   indented code after an indented starter). So a line that could start a
//!   block, after any leading spaces and tabs as the parser skips them, ends the
//!   scan without a boundary. Every other line is a table row, or, after a row
//!   of only `|` and whitespace (which has no cell and ends the table), text of
//!   a paragraph, which also ends at a blank line and is interrupted only by
//!   lines that end the scan.
//!
//! Lists, block quotes and HTML blocks of types 1-5 are not resumed.
use crate::decode::Decoded;

/// The kind of a top-level block whose end can be found lexically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `` ` `` or `~`, and the length of the opening marker run (at least three).
    Fence { marker: u8, length: usize },
    /// A GFM table.
    Table,
    /// An HTML block of CommonMark type 6 or 7.
    Html,
}

/// An open top-level block and the next source line still to be checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpenBlock {
    /// Source offset of the block's opening line.
    pub start: u64,
    pub kind: Kind,
    /// Every complete line from the opening line up to this source offset has
    /// been checked and belongs to the block.
    pub resume: u64,
}

/// Outcome of continuing a block scan through decoded text.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Scan {
    /// The text does not reach the resume point; nothing was learned.
    Gap,
    /// The block stays open at least up to the updated resume point.
    Open,
    /// The block ends; this source offset starts the next top-level unit.
    Closed(u64),
    /// A line may have started another block before the end was proven; the
    /// scan ends without a boundary.
    Abandoned,
}

/// What one complete line means for an open block.
enum Line {
    Inside,
    Ends,
    Abandon,
}

/// The opening marker of a fenced code block starting at `text`.
pub(crate) fn opening(text: &str) -> Option<(u8, usize)> {
    let body = text.trim_start_matches(' ');
    if text.len() - body.len() > 3 {
        return None;
    }
    let marker = *body.as_bytes().first()?;
    let length = body.bytes().take_while(|&byte| byte == marker).count();
    (matches!(marker, b'`' | b'~') && length >= 3).then_some((marker, length))
}

/// [`Kind::Html`] when the HTML block starting at `text` is of CommonMark type
/// 6 or 7, which ends at its first blank line. The other types end at closing
/// text and are not resumed: type 1 opens with `<script`, `<pre`, `<style` or
/// `<textarea` (any case) followed by a space, a tab, `>` or the end of the
/// line, type 2 with `<!--`, type 3 with `<?`, type 4 with `<!` and an ASCII
/// letter and type 5 with `<![CDATA[`.
pub(crate) fn html_block_kind(text: &str) -> Option<Kind> {
    let line = text.split('\n').next()?;
    let body = line.trim_start_matches(' ');
    if line.len() - body.len() > 3 {
        return None;
    }
    let rest = body.strip_prefix('<')?.as_bytes();
    for name in ["script", "pre", "style", "textarea"] {
        if rest
            .get(..name.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(name.as_bytes()))
            && matches!(rest.get(name.len()), None | Some(b' ' | b'\t' | b'>'))
        {
            return None;
        }
    }
    let other = rest.starts_with(b"!--")
        || rest.starts_with(b"?")
        || rest.starts_with(b"![CDATA[")
        || (rest.first() == Some(&b'!') && rest.get(1).is_some_and(u8::is_ascii_alphabetic));
    (!other).then_some(Kind::Html)
}

/// Spaces and tabs only (CR, LF and CRLF are decoded as one `\n`; vertical tab
/// and form feed are decoded as control pictures).
fn blank(line: &str) -> bool {
    line.bytes().all(|byte| byte == b' ' || byte == b'\t')
}

/// Whether `line` could start another block, as a heading, block quote, fence,
/// HTML block, list item, thematic break or setext underline. Leading spaces
/// and tabs are skipped, as a table row's parser skips them.
fn may_start_block(line: &str) -> bool {
    let body = line.trim_start_matches([' ', '\t']).as_bytes();
    match body.first() {
        Some(b'#' | b'>' | b'`' | b'~' | b'<' | b'-' | b'+' | b'*' | b'_' | b'=') => true,
        Some(b'0'..=b'9') => {
            let digits = body.iter().take_while(|byte| byte.is_ascii_digit()).count();
            digits <= 9
                && matches!(body.get(digits), Some(b'.' | b')'))
                && matches!(body.get(digits + 1), None | Some(b' ' | b'\t'))
        }
        _ => false,
    }
}

impl Kind {
    fn line(self, line: &str) -> Line {
        match self {
            Self::Fence { marker, length } if closes(marker, length, line) => Line::Ends,
            Self::Fence { .. } => Line::Inside,
            Self::Html | Self::Table if blank(line) => Line::Ends,
            Self::Table if may_start_block(line) => Line::Abandon,
            Self::Html | Self::Table => Line::Inside,
        }
    }
}

/// CommonMark closing fence: at most three spaces of indentation, at least as
/// many markers as the opening fence, then only spaces or tabs.
fn closes(marker: u8, length: usize, line: &str) -> bool {
    let body = line.trim_start_matches(' ');
    if line.len() - body.len() > 3 {
        return false;
    }
    let run = body.bytes().take_while(|&byte| byte == marker).count();
    run >= length
        && body[run..]
            .bytes()
            .all(|byte| byte == b' ' || byte == b'\t')
}

impl OpenBlock {
    /// Check the complete lines of `decoded` from the resume point.
    pub fn advance(&mut self, decoded: &Decoded) -> Scan {
        let Ok(index) = decoded
            .anchors
            .binary_search_by_key(&self.resume, |anchor| anchor.source_offset)
        else {
            return Scan::Gap;
        };
        let mut at = decoded.anchors[index].text_offset as usize;
        while let Some(length) = decoded.text.get(at..).and_then(|rest| rest.find('\n')) {
            let line = &decoded.text[at..at + length];
            let next = at + length + 1;
            match self.kind.line(line) {
                Line::Inside => {}
                Line::Ends => return decoded.source_at(next).map_or(Scan::Gap, Scan::Closed),
                Line::Abandon => return Scan::Abandoned,
            }
            match decoded.source_at(next) {
                Some(offset) => self.resume = offset,
                None => return Scan::Gap,
            }
            at = next;
        }
        Scan::Open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{Encoding, decode};

    fn decoded(text: &str, offset: u64) -> Decoded {
        decode(
            text.as_bytes(),
            offset,
            Encoding::Utf8,
            false,
            65536,
            usize::MAX,
            || false,
        )
        .unwrap()
    }

    fn open(kind: Kind, resume: u64) -> OpenBlock {
        OpenBlock {
            start: 0,
            kind,
            resume,
        }
    }

    #[test]
    fn finds_only_a_matching_closing_line() {
        assert_eq!(opening("````rust\n"), Some((b'`', 4)));
        assert_eq!(opening("   ~~~\n"), Some((b'~', 3)));
        assert_eq!(opening("    ```\n"), None);
        assert_eq!(opening("``\n"), None);
        let mut fence = open(
            Kind::Fence {
                marker: b'`',
                length: 4,
            },
            100,
        );
        let text = "```\n~~~~\n ````` x\n    ````\n  `````  \nafter\n";
        let closing = text.find("  `````  ").unwrap() + "  `````  \n".len();
        assert_eq!(
            fence.advance(&decoded(text, 100)),
            Scan::Closed(100 + closing as u64)
        );
    }

    #[test]
    fn an_open_fence_resumes_only_contiguously() {
        let mut fence = open(
            Kind::Fence {
                marker: b'`',
                length: 3,
            },
            10,
        );
        assert_eq!(fence.advance(&decoded("log\nmore lo", 50)), Scan::Gap);
        assert_eq!(fence.advance(&decoded("log\nmore lo", 10)), Scan::Open);
        assert_eq!(
            fence.resume, 14,
            "the incomplete last line is checked again"
        );
        assert_eq!(
            fence.advance(&decoded("more log\n```\nnext", 14)),
            Scan::Closed(27)
        );
    }

    #[test]
    fn only_html_blocks_of_types_six_and_seven_are_resumed() {
        for html in [
            "<div>\n",
            "   <DIV class=\"x\">\n",
            "</table>\n",
            "<custom-tag>\n",
            "<prefix>\n",
            "<scripts>\n",
        ] {
            assert_eq!(html_block_kind(html), Some(Kind::Html), "{html:?}");
        }
        for other in [
            "<pre>\n",
            "<PRE\n",
            "<script type=\"x\">\n",
            "<style>\n",
            "<textarea\tx>\n",
            "<!-- comment\n",
            "<?php\n",
            "<!DOCTYPE html>\n",
            "<![CDATA[\n",
            "    <div>\n",
        ] {
            assert_eq!(html_block_kind(other), None, "{other:?}");
        }
    }

    #[test]
    fn a_type_six_html_block_ends_after_its_first_blank_line() {
        let mut html = open(Kind::Html, 10);
        let text = "<p>a</p>\n  <span>b</span>\n # not a heading\n \t \nafter\n";
        let end = text.find(" \t \n").unwrap() + " \t \n".len();
        assert_eq!(
            html.advance(&decoded(text, 10)),
            Scan::Closed(10 + end as u64)
        );
    }

    #[test]
    fn a_table_ends_after_its_first_blank_line() {
        let mut table = open(Kind::Table, 0);
        let text = "| a | b |\nno pipe row\n\t| c | 1.5 |\n|\n\nafter\n";
        let end = text.find("\n\n").unwrap() + 2;
        assert_eq!(table.advance(&decoded(text, 0)), Scan::Closed(end as u64));
    }

    #[test]
    fn a_table_scan_is_abandoned_at_a_line_that_may_start_another_block() {
        for starter in [
            "- x",
            "> q",
            "```",
            "~~~",
            "# h",
            "1. x",
            "2) x",
            "1.",
            "<div>",
            "***",
            "___",
            "+ x",
            "===",
            "    - x",
            "\t> q",
            "        # h",
        ] {
            let mut table = open(Kind::Table, 0);
            let text = format!("| a | b |\n{starter}\n\n| c | d |\n");
            assert_eq!(
                table.advance(&decoded(&text, 0)),
                Scan::Abandoned,
                "{starter:?}"
            );
        }
        // A row of only `|` ends the table; the paragraph after it is interrupted by
        // the fence, whose blank line is not a boundary.
        let mut table = open(Kind::Table, 0);
        let text = "| a | b |\n | \n```\n\n# not a heading\n```\n";
        assert_eq!(table.advance(&decoded(text, 0)), Scan::Abandoned);
        // Digits that are not a list marker keep the table open.
        let mut table = open(Kind::Table, 0);
        assert_eq!(
            table.advance(&decoded("12.5 | x\n1234567890. y\n", 0)),
            Scan::Open
        );
    }

    #[test]
    fn a_table_or_html_scan_resumes_only_contiguously() {
        for kind in [Kind::Table, Kind::Html] {
            let mut block = open(kind, 10);
            assert_eq!(block.advance(&decoded("| x |\n| y", 50)), Scan::Gap);
            assert_eq!(block.advance(&decoded("| x |\n| y", 10)), Scan::Open);
            assert_eq!(
                block.resume, 16,
                "the incomplete last line is checked again"
            );
            assert_eq!(
                block.advance(&decoded("| y |\n\nnext", 16)),
                Scan::Closed(23)
            );
        }
    }
}
