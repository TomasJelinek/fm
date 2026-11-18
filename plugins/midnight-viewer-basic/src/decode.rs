use midnight_plugin_sdk::api::{
    viewer::{Anchor, MAX_PAGE_BYTES, MAX_SPANS, STYLE_INVALID, SourceRange, Span, TextPage},
    wire::MAX_ITEMS,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

pub fn detect(bytes: &[u8]) -> (Encoding, u64) {
    if bytes.starts_with(&[255, 254]) {
        (Encoding::Utf16Le, 2)
    } else if bytes.starts_with(&[254, 255]) {
        (Encoding::Utf16Be, 2)
    } else if bytes.starts_with(&[239, 187, 191]) {
        (Encoding::Utf8, 3)
    } else {
        (Encoding::Utf8, 0)
    }
}

// 8192 boundary entries fit the aggregate CBOR item budget as well as MAX_ANCHORS.
pub const MAX_UNITS: usize = 8191;
/// Largest positional read of one viewport.
pub const READ_BYTES: usize = 8192;
/// Escape runs marked as invalid per page. Every page item fits the wire budget:
/// 21 page items, 9 per span and 5 per anchor.
pub const MAX_INVALID_SPANS: usize = 2048;
const _: () = assert!(21 + 9 * MAX_INVALID_SPANS + 5 * (MAX_UNITS + 1) <= MAX_ITEMS);
const _: () = assert!(MAX_INVALID_SPANS <= MAX_SPANS);
// A unit displays at most six bytes, so a page never needs a text byte budget.
const _: () = assert!(6 * MAX_UNITS <= MAX_PAGE_BYTES);

/// One decoded source unit. Escapes display the original byte, UTF-16 word or code point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// A scalar displayed as itself; CR, LF and CRLF decode to one '\n'.
    Char(char),
    /// An invalid byte or a C0/C1/DEL control, displayed as `\xNN`.
    Byte(u8),
    /// A malformed UTF-16 code unit or an explicit bidirectional control, displayed as
    /// `\uNNNN`.
    Word(u16),
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

impl Unit {
    pub fn is_escape(self) -> bool {
        !matches!(self, Self::Char(_))
    }

    /// The displayed characters, without allocating.
    pub fn visible(self) -> ([char; 6], usize) {
        let hex = |value: u16, shift: u16| char::from(HEX[usize::from((value >> shift) & 15)]);
        match self {
            Self::Char(c) => ([c, '\0', '\0', '\0', '\0', '\0'], 1),
            Self::Byte(byte) => {
                let byte = u16::from(byte);
                (['\\', 'x', hex(byte, 4), hex(byte, 0), '\0', '\0'], 4)
            }
            Self::Word(word) => (
                [
                    '\\',
                    'u',
                    hex(word, 12),
                    hex(word, 8),
                    hex(word, 4),
                    hex(word, 0),
                ],
                6,
            ),
        }
    }

    fn push_to(self, text: &mut String) {
        let (characters, count) = self.visible();
        text.extend(&characters[..count]);
    }
}

/// Decoded units of `bytes` with their byte ranges. Stops before a unit that needs
/// more input unless `eof`; a CR waits for its possible LF in the same way.
pub fn units(bytes: &[u8], encoding: Encoding, eof: bool) -> Units<'_> {
    Units {
        bytes,
        encoding,
        eof,
        used: 0,
    }
}

pub struct Units<'a> {
    bytes: &'a [u8],
    encoding: Encoding,
    eof: bool,
    used: usize,
}

impl Iterator for Units<'_> {
    /// The unit and the exclusive end of its bytes.
    type Item = (Unit, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let rest = &self.bytes[self.used..];
        let (mut decoded, mut count) = unit(rest, self.encoding, self.eof)?;
        if decoded == Unit::Char('\r') {
            match unit(&rest[count..], self.encoding, self.eof) {
                // An incomplete look-ahead may still be the LF of this CRLF.
                None if !self.eof => return None,
                Some((Unit::Char('\n'), length)) => count += length,
                _ => {}
            }
            decoded = Unit::Char('\n');
        }
        self.used += count;
        Some((decoded, self.used))
    }
}

/// Decodes at most MAX_UNITS units with exact source anchors. Escaped units are
/// marked STYLE_INVALID (adjacent escapes share one span) up to MAX_INVALID_SPANS.
pub fn page(bytes: &[u8], encoding: Encoding, offset: u64, eof: bool) -> TextPage {
    let mut text = String::with_capacity(bytes.len().min(MAX_UNITS));
    let mut anchors = Vec::with_capacity(bytes.len().min(MAX_UNITS) + 1);
    anchors.push(Anchor {
        text_offset: 0,
        source_offset: offset,
    });
    let mut spans: Vec<Span> = Vec::new();
    let mut used = 0;
    for (decoded, end) in units(bytes, encoding, eof) {
        if anchors.len() > MAX_UNITS {
            break;
        }
        let Some(source_offset) = offset.checked_add(end as u64) else {
            break;
        };
        let start = text.len() as u32;
        decoded.push_to(&mut text);
        let text_offset = text.len() as u32;
        if decoded.is_escape() {
            if let Some(span) = spans.last_mut().filter(|span| span.end == start) {
                span.end = text_offset;
            } else if spans.len() < MAX_INVALID_SPANS {
                spans.push(Span {
                    start,
                    end: text_offset,
                    role: STYLE_INVALID,
                    flags: 0,
                });
            }
        }
        used = end;
        anchors.push(Anchor {
            text_offset,
            source_offset,
        });
    }
    let has_after = !eof || used < bytes.len();
    let partial_line = (offset != 0) || (has_after && !text.ends_with('\n'));
    TextPage {
        revision: 0,
        source: SourceRange {
            offset,
            length: used as u64,
        },
        text,
        spans,
        anchors,
        has_before: offset != 0,
        has_after,
        partial_line,
    }
}

fn unit(bytes: &[u8], encoding: Encoding, eof: bool) -> Option<(Unit, usize)> {
    let &first = bytes.first()?;
    match encoding {
        Encoding::Utf8 => {
            let len = match first {
                0..=127 => 1,
                194..=223 => 2,
                224..=239 => 3,
                240..=244 => 4,
                _ => return Some((Unit::Byte(first), 1)),
            };
            if bytes.len() < len && !eof {
                return None;
            }
            if let Some(c) = bytes
                .get(..len)
                .and_then(|scalar| std::str::from_utf8(scalar).ok())
                .and_then(|scalar| scalar.chars().next())
            {
                return Some((visible(c), len));
            }
            Some((Unit::Byte(first), 1))
        }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            if bytes.len() < 2 {
                return eof.then_some((Unit::Byte(first), 1));
            }
            let word = |b: &[u8]| {
                if encoding == Encoding::Utf16Le {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            };
            let w = word(bytes);
            if (0xD800..=0xDBFF).contains(&w) {
                if bytes.len() < 4 && !eof {
                    return None;
                }
                if bytes.len() >= 4 {
                    let low = word(&bytes[2..]);
                    if (0xDC00..=0xDFFF).contains(&low)
                        && let Some(c) = char::from_u32(
                            0x10000 + ((u32::from(w) - 0xD800) << 10) + (u32::from(low) - 0xDC00),
                        )
                    {
                        return Some((visible(c), 4));
                    }
                }
                return Some((Unit::Word(w), 2));
            }
            Some((char::from_u32(w.into()).map_or(Unit::Word(w), visible), 2))
        }
    }
}

fn visible(c: char) -> Unit {
    let code = u32::from(c);
    match code {
        // Unicode Cc controls (a stable set) are exactly C0, DEL and C1, all below U+0100.
        // They keep the `\xNN` form.
        0..=0x1f | 0x7f..=0x9f if !matches!(c, '\r' | '\n' | '\t') => Unit::Byte(code as u8),
        // The explicit bidirectional embedding, override and isolate controls would let file
        // content reorder the displayed text ("Trojan Source"). They are named losslessly as
        // `\uNNNN`. Implicit direction marks (LRM, RLM, ALM) stay ordinary text for RTL prose.
        0x202a..=0x202e | 0x2066..=0x2069 => Unit::Word(code as u16),
        _ => Unit::Char(c),
    }
}
