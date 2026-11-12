use midnight_plugin_sdk::api::viewer::{Anchor, MAX_PAGE_BYTES, SourceRange, TextPage};

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
pub const READ_BYTES: usize = 8192;

pub fn page(
    bytes: &[u8],
    encoding: Encoding,
    offset: u64,
    eof: bool,
    max_bytes: usize,
    max_lines: usize,
) -> TextPage {
    let mut text = String::new();
    let mut anchors = vec![Anchor {
        text_offset: 0,
        source_offset: offset,
    }];
    let mut used = 0;
    let mut lines = 0;
    while used < bytes.len() && anchors.len() <= MAX_UNITS {
        let Some((mut decoded, mut count)) = unit(&bytes[used..], encoding, eof) else {
            break;
        };
        if decoded == "\r" {
            if used + count == bytes.len() && !eof {
                break;
            }
            if let Some((next, length)) = unit(&bytes[used + count..], encoding, eof)
                && next == "\n"
            {
                count += length;
            }
            decoded = "\n".into();
        }
        if text.len() + decoded.len() > max_bytes.min(MAX_PAGE_BYTES)
            || offset.checked_add((used + count) as u64).is_none()
        {
            break;
        }
        text.push_str(&decoded);
        used += count;
        anchors.push(Anchor {
            text_offset: text.len() as u32,
            source_offset: offset + used as u64,
        });
        if decoded == "\n" {
            lines += 1;
            if lines >= max_lines {
                break;
            }
        }
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
        spans: vec![],
        anchors,
        has_before: offset != 0,
        has_after,
        partial_line,
    }
}

fn unit(bytes: &[u8], encoding: Encoding, eof: bool) -> Option<(String, usize)> {
    let &first = bytes.first()?;
    match encoding {
        Encoding::Utf8 => {
            let len = match first {
                0..=127 => 1,
                194..=223 => 2,
                224..=239 => 3,
                240..=244 => 4,
                _ => return Some((format!("\\x{first:02X}"), 1)),
            };
            if bytes.len() < len && !eof {
                return None;
            }
            if bytes.len() >= len
                && let Ok(s) = std::str::from_utf8(&bytes[..len])
            {
                return Some((visible(s.chars().next()?), len));
            }
            Some((format!("\\x{first:02X}"), 1))
        }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            if bytes.len() < 2 {
                return eof.then(|| (format!("\\x{first:02X}"), 1));
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
                    if (0xDC00..=0xDFFF).contains(&low) {
                        let c = char::from_u32(
                            0x10000 + ((w as u32 - 0xD800) << 10) + (low as u32 - 0xDC00),
                        )?;
                        return Some((visible(c), 4));
                    }
                }
                return Some((format!("\\u{w:04X}"), 2));
            }
            Some((
                char::from_u32(w.into())
                    .map(visible)
                    .unwrap_or_else(|| format!("\\u{w:04X}")),
                2,
            ))
        }
    }
}

fn visible(c: char) -> String {
    if c.is_control() && !matches!(c, '\r' | '\n' | '\t') {
        if c as u32 <= 255 {
            format!("\\x{:02X}", c as u32)
        } else {
            format!("\\u{:04X}", c as u32)
        }
    } else {
        c.to_string()
    }
}
