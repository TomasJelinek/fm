//! Regular expression content search: the decoded stream is split into lines and
//! each line is matched on its own, like grep. Only one bounded line is retained.
use midnight_core::FindContentQuery;
use midnight_platform_common::content_pattern::LinePattern;

/// A longer line is matched in windows of this size that overlap by `OVERLAP`
/// bytes, so retained text stays bounded; a match that spans a window boundary
/// and is longer than the overlap can be missed.
pub(super) const MAX_LINE_BYTES: usize = 1024 * 1024;
pub(super) const OVERLAP: usize = 4 * 1024;

pub(super) struct Lines {
    pattern: LinePattern,
    line: String,
    found: bool,
}

impl Lines {
    pub(super) fn new(query: &FindContentQuery) -> Result<Self, String> {
        Ok(Self {
            pattern: LinePattern::new(&query.text, query.case_sensitive, query.whole_words)?,
            line: String::new(),
            found: false,
        })
    }

    pub(super) fn reset(&mut self) {
        self.line.clear();
        if self.line.capacity() > 2 * MAX_LINE_BYTES {
            self.line.shrink_to(MAX_LINE_BYTES);
        }
        self.found = false;
    }

    pub(super) fn consume(&mut self, text: &str) {
        if self.found {
            return;
        }
        let mut rest = text;
        while let Some(end) = rest.find('\n') {
            self.line.push_str(&rest[..end]);
            if self.matches() {
                self.found = true;
                self.line.clear();
                return;
            }
            self.line.clear();
            rest = &rest[end + 1..];
        }
        self.line.push_str(rest);
        if self.line.len() >= MAX_LINE_BYTES {
            if self.matches() {
                self.found = true;
                self.line.clear();
                return;
            }
            let mut keep = self.line.len() - OVERLAP;
            while !self.line.is_char_boundary(keep) {
                keep -= 1;
            }
            self.line.drain(..keep);
        }
    }

    fn matches(&self) -> bool {
        // CRLF files: the carriage return is not part of the line.
        let line = self.line.strip_suffix('\r').unwrap_or(&self.line);
        self.pattern.is_match(line)
    }

    /// The last line has no terminator; an empty remainder is no line.
    pub(super) fn finish(&mut self) -> bool {
        if !self.found && !self.line.is_empty() {
            self.found = self.matches();
        }
        self.line.clear();
        self.found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str, case_sensitive: bool, whole_words: bool) -> Lines {
        Lines::new(&FindContentQuery {
            text: text.into(),
            case_sensitive,
            whole_words,
            regex: true,
        })
        .unwrap()
    }

    fn search(lines: &mut Lines, chunks: &[&str]) -> bool {
        lines.reset();
        for chunk in chunks {
            lines.consume(chunk);
        }
        lines.finish()
    }

    #[test]
    fn matches_within_lines_across_chunks_and_never_across_lines() {
        let mut pattern = lines(r"^fn \w+\(\)$", true, false);
        assert!(search(&mut pattern, &["struct A;\r\nfn ma", "in()\r\n"]));
        assert!(!search(&mut pattern, &["fn main(", "\n)"]));
        let mut across = lines("alpha.beta", false, false);
        assert!(!search(&mut across, &["alpha\nbeta"]));
        assert!(search(&mut across, &["ALPHA beta"]));
    }

    #[test]
    fn whole_words_and_case_and_last_line_without_newline() {
        let mut words = lines("cat|dog", false, true);
        assert!(!search(&mut words, &["concatenate\ndogma"]));
        assert!(search(&mut words, &["hot\nDOG"]));
        let mut exact = lines("Dog", true, false);
        assert!(!search(&mut exact, &["dog"]));
        let mut empty = lines("^$", false, false);
        assert!(!search(&mut empty, &["text\n"]), "no phantom last line");
        assert!(search(&mut empty, &["a\n\nb"]));
    }

    #[test]
    fn a_very_long_line_stays_bounded_and_still_matches_in_windows() {
        let mut pattern = lines("needle", false, false);
        pattern.reset();
        let filler = "x".repeat(64 * 1024);
        for _ in 0..40 {
            pattern.consume(&filler);
            assert!(pattern.line.len() < MAX_LINE_BYTES + filler.len());
        }
        pattern.consume("needle");
        assert!(pattern.finish());
    }
}
