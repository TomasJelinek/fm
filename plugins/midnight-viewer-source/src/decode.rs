//! Bounded decoder with an exact source boundary for each displayed scalar.
use midnight_plugin_sdk::api::viewer::{Anchor, SourceRange, TextPage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Le,
    Be,
}

pub fn encoding(bytes: &[u8]) -> (Encoding, usize) {
    if bytes.starts_with(&[0xff, 0xfe]) {
        (Encoding::Le, 2)
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        (Encoding::Be, 2)
    } else if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        (Encoding::Utf8, 3)
    } else {
        (Encoding::Utf8, 0)
    }
}

/// The (host) cancellation callback is polled once per this many scalars.
const POLL_SCALARS: usize = 4096;

#[derive(Clone, Debug)]
pub struct Decoded {
    pub text: String,
    pub anchors: Vec<Anchor>,
    pub consumed: usize,
}

/// Ends decoding once a forward page is complete: `bytes` text bytes or
/// `lines` newlines follow the first scalar boundary at or after `source`.
#[derive(Clone, Copy, Debug)]
pub struct Tail {
    pub source: u64,
    pub bytes: usize,
    pub lines: usize,
}

/// Incomplete scalars, and a CR whose next unit is still incomplete, are left
/// unread unless EOF is known.
pub fn decode(
    bytes: &[u8],
    offset: u64,
    encoding: Encoding,
    eof: bool,
    max_bytes: usize,
    max_lines: usize,
    cancelled: impl Fn() -> bool,
) -> Option<Decoded> {
    decode_until(
        bytes, offset, encoding, eof, max_bytes, max_lines, None, 0, cancelled,
    )
}

#[allow(clippy::too_many_arguments)]
fn decode_until(
    bytes: &[u8],
    offset: u64,
    encoding: Encoding,
    eof: bool,
    max_bytes: usize,
    max_lines: usize,
    tail: Option<Tail>,
    lookahead: usize,
    cancelled: impl Fn() -> bool,
) -> Option<Decoded> {
    offset.checked_add(bytes.len() as u64)?;
    if cancelled() {
        return None;
    }
    let limit = max_bytes.min(64 * 1024);
    // Each scalar consumes at least one source byte and emits at least one text
    // byte, so this reserves every anchor once instead of growing geometrically.
    let mut scalars = bytes.len().min(limit);
    if let Some(tail) = tail {
        let before = usize::try_from(tail.source.saturating_sub(offset)).unwrap_or(usize::MAX);
        scalars = scalars.min(
            before
                .saturating_add(tail.bytes)
                .saturating_add(lookahead)
                .saturating_add(1),
        );
    }
    let mut decoded = Decoded {
        text: String::with_capacity(limit.min(bytes.len())),
        anchors: Vec::with_capacity(scalars + 1),
        consumed: 0,
    };
    decoded.anchors.push(Anchor {
        text_offset: 0,
        source_offset: offset,
    });
    // Text offset and newline count since the tail boundary, once reached.
    let mut page = tail.filter(|tail| offset >= tail.source).map(|_| (0, 0));
    let mut after_page = None;
    let mut lines = 0;
    while decoded.consumed < bytes.len() && lines < max_lines {
        if decoded.anchors.len().is_multiple_of(POLL_SCALARS) && cancelled() {
            return None;
        }
        let at = decoded.consumed;
        let Some((mut ch, mut size)) = scalar(&bytes[at..], encoding, eof) else {
            break;
        };
        if ch == '\r' {
            match scalar(&bytes[at + size..], encoding, eof) {
                // An incomplete look-ahead may still be the LF of this CRLF.
                None if !eof => break,
                Some(('\n', next)) => size += next,
                _ => {}
            }
            ch = '\n';
        } else if ch.is_control() && ch != '\n' && ch != '\t' {
            ch = if ch <= '\u{1f}' {
                char::from_u32(0x2400 + ch as u32).unwrap()
            } else if ch == '\u{7f}' {
                '\u{2421}'
            } else {
                '\u{fffd}'
            };
        } else if matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
            // Like C1 controls, the explicit bidirectional embedding, override and isolate
            // controls display as U+FFFD, so file content cannot reorder the displayed text
            // ("Trojan Source"). One replacement keeps one boundary per displayed scalar.
            // Implicit direction marks (LRM, RLM, ALM) remain for ordinary RTL text.
            ch = '\u{fffd}';
        }
        if decoded.text.len() + ch.len_utf8() > limit
            || after_page.is_some_and(|end| decoded.text.len() + ch.len_utf8() > end)
        {
            break;
        }
        decoded.text.push(ch);
        decoded.consumed += size;
        decoded.anchors.push(Anchor {
            text_offset: decoded.text.len() as u32,
            source_offset: offset + decoded.consumed as u64,
        });
        if ch == '\n' {
            lines += 1;
        }
        if let Some(end) = after_page {
            if decoded.text.len() >= end {
                break;
            }
        } else if let Some(tail) = tail {
            if let Some((start, newlines)) = &mut page {
                *newlines += usize::from(ch == '\n');
                if decoded.text.len() - *start >= tail.bytes || *newlines >= tail.lines {
                    if lookahead == 0 {
                        break;
                    }
                    after_page = Some(decoded.text.len().saturating_add(lookahead));
                }
            } else if offset + decoded.consumed as u64 >= tail.source {
                page = Some((decoded.text.len(), 0));
            }
        }
    }
    Some(decoded)
}

fn scalar(bytes: &[u8], encoding: Encoding, eof: bool) -> Option<(char, usize)> {
    let &first = bytes.first()?;
    if encoding == Encoding::Utf8 {
        let width = match first {
            0..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return Some(('�', 1)),
        };
        if bytes.len() < width {
            return eof.then_some(('�', 1));
        }
        return Some(match std::str::from_utf8(&bytes[..width]) {
            Ok(text) => (text.chars().next().unwrap(), width),
            Err(_) => ('�', 1),
        });
    }
    if bytes.len() < 2 {
        return eof.then_some(('�', 1));
    }
    let unit = |pair: &[u8]| {
        if encoding == Encoding::Le {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    };
    let first = unit(bytes);
    if (0xd800..=0xdbff).contains(&first) {
        if bytes.len() < 4 {
            return eof.then_some(('�', 2));
        }
        let second = unit(&bytes[2..]);
        if (0xdc00..=0xdfff).contains(&second) {
            let value = 0x10000 + ((first as u32 - 0xd800) << 10) + second as u32 - 0xdc00;
            return Some((char::from_u32(value).unwrap(), 4));
        }
        return Some(('�', 2));
    }
    Some((char::from_u32(first as u32).unwrap_or('�'), 2))
}

impl Decoded {
    pub fn source_at(&self, text: usize) -> Option<u64> {
        self.anchors
            .binary_search_by_key(&(text as u32), |a| a.text_offset)
            .ok()
            .map(|i| self.anchors[i].source_offset)
    }

    pub fn page(self, revision: u64, eof: bool) -> TextPage {
        let start = self.anchors.first().unwrap().source_offset;
        let end = self.anchors.last().unwrap().source_offset;
        let partial_line = !eof && !self.text.ends_with('\n');
        TextPage {
            revision,
            source: SourceRange {
                offset: start,
                length: end - start,
            },
            text: self.text,
            spans: Vec::new(),
            anchors: self.anchors,
            has_before: start != 0,
            has_after: !eof,
            partial_line,
        }
    }
}

/// Four bytes of look-behind after UTF-16 word alignment suffice for a split
/// UTF-8 scalar, UTF-16 surrogate pair, or CRLF. The requested anchor is kept
/// separately: rounding the read position must never round the user's limit.
pub fn lookbehind(anchor: u64, bom: u64, encoding: Encoding) -> u64 {
    let mut aligned = anchor.max(bom);
    if encoding != Encoding::Utf8 {
        aligned -= (aligned - bom) % 2;
    }
    aligned.saturating_sub(4).max(bom)
}

/// Decode an arbitrary read whose initial look-behind may itself split a scalar.
/// Discard only that leading fragment; callers select the requested boundary
/// after decoding, so genuine malformed bytes at that boundary remain visible.
/// A `tail` stops decoding once the forward page after its source is complete.
pub fn fragment(
    bytes: &[u8],
    location: (u64, Encoding, bool),
    eof: bool,
    max_bytes: usize,
    tail: Option<Tail>,
    cancelled: impl Fn() -> bool,
) -> Option<Decoded> {
    fragment_with_lookahead(bytes, location, eof, max_bytes, tail, 0, cancelled)
}

/// Streaming syntax needs only this bounded suffix beyond the selected page.
pub(crate) fn fragment_with_lookahead(
    bytes: &[u8],
    location: (u64, Encoding, bool),
    eof: bool,
    max_bytes: usize,
    tail: Option<Tail>,
    lookahead: usize,
    cancelled: impl Fn() -> bool,
) -> Option<Decoded> {
    let (offset, encoding, known_boundary) = location;
    let skip = if known_boundary {
        0
    } else if encoding == Encoding::Utf8 {
        bytes
            .iter()
            .take(3)
            .take_while(|byte| **byte & 0xc0 == 0x80)
            .count()
    } else if bytes.len() >= 2 {
        let unit = if encoding == Encoding::Le {
            u16::from_le_bytes([bytes[0], bytes[1]])
        } else {
            u16::from_be_bytes([bytes[0], bytes[1]])
        };
        if (0xdc00..=0xdfff).contains(&unit) {
            2
        } else {
            0
        }
    } else {
        0
    };
    let mut decoded = decode_until(
        &bytes[skip..],
        offset.checked_add(skip as u64)?,
        encoding,
        eof,
        max_bytes,
        usize::MAX,
        tail,
        lookahead,
        cancelled,
    )?;
    decoded.consumed += skip;
    Some(decoded)
}

/// The nearest preceding displayed scalars, counting a trailing newline as the
/// end of its preceding line rather than as an extra empty line.
pub fn suffix_start(text: &str, end: usize, max_bytes: usize, max_lines: usize) -> usize {
    let mut start = end;
    let mut lines = 0;
    for (at, ch) in text[..end].char_indices().rev() {
        if end - at > max_bytes || max_lines == 0 {
            break;
        }
        if ch == '\n' && at + 1 != end {
            lines += 1;
            if lines >= max_lines {
                break;
            }
        }
        start = at;
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forward_tail_keeps_only_the_requested_streaming_lookahead() {
        let bytes = b"prefix\nline0\nline1\nline2\n";
        let tail = Some(Tail {
            source: 7,
            bytes: 8192,
            lines: 1,
        });
        let page = fragment(bytes, (0, Encoding::Utf8, true), true, 65536, tail, || {
            false
        })
        .unwrap();
        assert_eq!(page.text, "prefix\nline0\n");
        let with_context = fragment_with_lookahead(
            bytes,
            (0, Encoding::Utf8, true),
            true,
            65536,
            tail,
            3,
            || false,
        )
        .unwrap();
        assert_eq!(with_context.text, "prefix\nline0\nlin");
        assert_eq!(with_context.consumed, 16);
        assert_eq!(with_context.anchors.last().unwrap().source_offset, 16);
    }
}
