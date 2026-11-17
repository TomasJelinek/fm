//! Resynchronization after a top-level fenced code block longer than a read.
//!
//! A parse can prove boundaries only inside its own read, so such a block
//! would otherwise end checkpoint discovery for the rest of the document. Its
//! end is purely lexical: the first later line that is a closing fence of the
//! same marker. Plain reads that continue the scan contiguously find that line
//! and prove the boundary after it.
use crate::decode::Decoded;

/// An open top-level fence and the next source line still to be checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpenFence {
    /// Source offset of the opening fence line.
    pub start: u64,
    /// `` ` `` or `~`.
    pub marker: u8,
    /// Length of the opening marker run (at least three).
    pub length: usize,
    /// Every complete line from the opening line up to this source offset
    /// has been checked and is fence content.
    pub resume: u64,
}

/// Outcome of continuing a fence scan through decoded text.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Scan {
    /// The text does not reach the resume point; nothing was learned.
    Gap,
    /// The fence stays open at least up to the updated resume point.
    Open,
    /// The fence closes; this source offset starts the next top-level unit.
    Closed(u64),
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

impl OpenFence {
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
            if self.closes(line) {
                return decoded.source_at(next).map_or(Scan::Gap, Scan::Closed);
            }
            match decoded.source_at(next) {
                Some(offset) => self.resume = offset,
                None => return Scan::Gap,
            }
            at = next;
        }
        Scan::Open
    }

    /// CommonMark closing fence: at most three spaces of indentation, at least
    /// as many markers as the opening fence, then only spaces or tabs.
    fn closes(&self, line: &str) -> bool {
        let body = line.trim_start_matches(' ');
        if line.len() - body.len() > 3 {
            return false;
        }
        let run = body.bytes().take_while(|&byte| byte == self.marker).count();
        run >= self.length
            && body[run..]
                .bytes()
                .all(|byte| byte == b' ' || byte == b'\t')
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

    #[test]
    fn finds_only_a_matching_closing_line() {
        assert_eq!(opening("````rust\n"), Some((b'`', 4)));
        assert_eq!(opening("   ~~~\n"), Some((b'~', 3)));
        assert_eq!(opening("    ```\n"), None);
        assert_eq!(opening("``\n"), None);
        let mut fence = OpenFence {
            start: 0,
            marker: b'`',
            length: 4,
            resume: 100,
        };
        let text = "```\n~~~~\n ````` x\n    ````\n  `````  \nafter\n";
        let closing = text.find("  `````  ").unwrap() + "  `````  \n".len();
        assert_eq!(
            fence.advance(&decoded(text, 100)),
            Scan::Closed(100 + closing as u64)
        );
    }

    #[test]
    fn an_open_fence_resumes_only_contiguously() {
        let mut fence = OpenFence {
            start: 0,
            marker: b'`',
            length: 3,
            resume: 10,
        };
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
}
