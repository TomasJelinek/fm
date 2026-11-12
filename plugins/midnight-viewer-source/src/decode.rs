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

#[derive(Clone, Debug)]
pub struct Decoded {
    pub text: String,
    pub anchors: Vec<Anchor>,
    pub consumed: usize,
}

/// Incomplete scalars and CR are left unread unless EOF is known.
pub fn decode(
    bytes: &[u8],
    offset: u64,
    encoding: Encoding,
    eof: bool,
    max_bytes: usize,
    max_lines: usize,
    cancelled: impl Fn() -> bool,
) -> Option<Decoded> {
    offset.checked_add(bytes.len() as u64)?;
    if cancelled() {
        return None;
    }
    let limit = max_bytes.min(64 * 1024);
    let mut decoded = Decoded {
        text: String::with_capacity(limit.min(bytes.len())),
        anchors: vec![Anchor {
            text_offset: 0,
            source_offset: offset,
        }],
        consumed: 0,
    };
    let mut lines = 0;
    while decoded.consumed < bytes.len() && lines < max_lines {
        if decoded.anchors.len().is_multiple_of(128) && cancelled() {
            return None;
        }
        let at = decoded.consumed;
        let Some((mut ch, mut size)) = scalar(&bytes[at..], encoding, eof) else {
            break;
        };
        if ch == '\r' {
            if at + size == bytes.len() && !eof {
                break;
            }
            if let Some(('\n', next)) = scalar(&bytes[at + size..], encoding, eof) {
                size += next;
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
        }
        if decoded.text.len() + ch.len_utf8() > limit {
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
pub fn fragment(
    bytes: &[u8],
    location: (u64, Encoding, bool),
    eof: bool,
    max_bytes: usize,
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
    let mut decoded = decode(
        &bytes[skip..],
        offset.checked_add(skip as u64)?,
        encoding,
        eof,
        max_bytes,
        usize::MAX,
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
