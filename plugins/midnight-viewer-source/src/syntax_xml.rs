//! XML 1.0 lexical colouring only: no validation, entity expansion or external reads.
use midnight_plugin_sdk::api::viewer::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct State {
    mode: Mode,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Text,
    Tag {
        processing: bool,
        first: bool,
        closing: bool,
    },
    Name {
        processing: bool,
        role: u16,
    },
    Quote {
        quote: u8,
        processing: bool,
    },
    Comment {
        declaration: Option<u8>,
        matched: u8,
    },
    Cdata {
        matched: u8,
    },
    Declaration {
        depth: u8,
        quote: Option<u8>,
    },
    Reference {
        length: u64,
        kind: Reference,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reference {
    Start,
    Numeric,
    HexStart,
    Decimal,
    Hex,
    Name,
}

impl State {
    pub(super) fn pending_style(&self) -> bool {
        self.pending_span().is_some()
    }

    /// Entity validity is unresolved until its semicolon or an invalid scalar.
    pub(super) fn pending_span(&self) -> Option<(u64, u64)> {
        match self.mode {
            Mode::Reference { length, .. } => Some((length, 0)),
            _ => None,
        }
    }
}

pub(super) fn scan(
    text: &str,
    state: &mut State,
    styles: &mut crate::styles::Styles,
    cancelled: impl Fn() -> bool,
) -> Option<()> {
    (stream(text, state, styles, true, cancelled)? == text.len()).then_some(())
}

/// Complete scalar prefixes use compact exact continuation state. Only a fixed
/// XML opening delimiter, at most eight bytes, may remain with the caller.
pub(super) fn stream(
    text: &str,
    state: &mut State,
    styles: &mut crate::styles::Styles,
    final_chunk: bool,
    cancelled: impl Fn() -> bool,
) -> Option<usize> {
    if text.len() > 65_536 || cancelled() {
        return None;
    }
    let mut current = *state;
    let bytes = text.as_bytes();
    let mut at = 0;
    let mut units = 0_u32;
    while at < bytes.len() {
        if units.is_multiple_of(128) && cancelled() {
            return None;
        }
        units = units.wrapping_add(1);
        let start = at;
        let tail = &bytes[at..];
        let byte = bytes[at];
        let ch = text[at..].chars().next()?;
        let width = ch.len_utf8();
        let role = match current.mode {
            Mode::Text => {
                if byte == b'<'
                    && !final_chunk
                    && [b"<!--".as_slice(), b"<![CDATA[", b"<?", b"</"]
                        .iter()
                        .any(|open| open.starts_with(tail) && tail.len() < open.len())
                {
                    break;
                }
                if tail.starts_with(b"<!--") {
                    at += 4;
                    current.mode = Mode::Comment {
                        declaration: None,
                        matched: 0,
                    };
                    Some(STYLE_COMMENT)
                } else if tail.starts_with(b"<![CDATA[") {
                    at += 9;
                    current.mode = Mode::Cdata { matched: 0 };
                    Some(STYLE_STRING)
                } else if tail.starts_with(b"<!") {
                    at += 2;
                    current.mode = Mode::Declaration {
                        depth: 0,
                        quote: None,
                    };
                    Some(STYLE_KEYWORD)
                } else if byte == b'<' {
                    at += 1;
                    let processing = bytes.get(at) == Some(&b'?');
                    if processing || bytes.get(at) == Some(&b'/') {
                        at += 1;
                    }
                    current.mode = Mode::Tag {
                        processing,
                        first: true,
                        closing: false,
                    };
                    Some(STYLE_OPERATOR)
                } else if byte == b'&' {
                    at += 1;
                    current.mode = Mode::Reference {
                        length: 1,
                        kind: Reference::Start,
                    };
                    None
                } else {
                    at += width;
                    None
                }
            }
            Mode::Tag {
                processing,
                first,
                closing,
            } => {
                if byte == b'>' && (!processing || closing) {
                    at += 1;
                    current.mode = Mode::Text;
                    Some(STYLE_OPERATOR)
                } else if matches!(byte, b'\'' | b'"') {
                    at += 1;
                    current.mode = Mode::Quote {
                        quote: byte,
                        processing,
                    };
                    Some(STYLE_STRING)
                } else if name_start(ch) {
                    current.mode = Mode::Name {
                        processing,
                        role: if first { STYLE_KEYWORD } else { STYLE_TYPE },
                    };
                    continue;
                } else {
                    at += width;
                    current.mode = Mode::Tag {
                        processing,
                        first,
                        closing: byte == if processing { b'?' } else { b'/' },
                    };
                    matches!(byte, b'=' | b'?' | b'/' | b'>').then_some(STYLE_OPERATOR)
                }
            }
            Mode::Name { processing, role } => {
                if name_char(ch) {
                    at += width;
                    Some(role)
                } else {
                    current.mode = Mode::Tag {
                        processing,
                        first: false,
                        closing: false,
                    };
                    continue;
                }
            }
            Mode::Quote { quote, processing } => {
                at += width;
                if byte == quote {
                    current.mode = Mode::Tag {
                        processing,
                        first: false,
                        closing: false,
                    };
                }
                Some(STYLE_STRING)
            }
            Mode::Comment {
                declaration,
                mut matched,
            } => {
                at += width;
                if byte == b'>' && matched == 2 {
                    current.mode = declaration
                        .map_or(Mode::Text, |depth| Mode::Declaration { depth, quote: None });
                } else {
                    matched = if byte == b'-' {
                        (matched + 1).min(2)
                    } else {
                        0
                    };
                    current.mode = Mode::Comment {
                        declaration,
                        matched,
                    };
                }
                Some(STYLE_COMMENT)
            }
            Mode::Cdata { mut matched } => {
                at += width;
                if byte == b'>' && matched == 2 {
                    current.mode = Mode::Text;
                } else {
                    matched = if byte == b']' {
                        (matched + 1).min(2)
                    } else {
                        0
                    };
                    current.mode = Mode::Cdata { matched };
                }
                Some(STYLE_STRING)
            }
            Mode::Declaration {
                mut depth,
                mut quote,
            } => {
                if let Some(delimiter) = quote {
                    if byte == delimiter {
                        quote = None;
                    }
                } else if byte == b'<'
                    && !final_chunk
                    && b"<!--".starts_with(tail)
                    && tail.len() < 4
                {
                    break;
                } else if tail.starts_with(b"<!--") {
                    at += 4;
                    styles.push(start, at, STYLE_COMMENT);
                    current.mode = Mode::Comment {
                        declaration: Some(depth),
                        matched: 0,
                    };
                    continue;
                } else {
                    match byte {
                        b'\'' | b'"' => quote = Some(byte),
                        b'[' => {
                            if depth == 16 {
                                return None;
                            }
                            depth += 1;
                        }
                        b']' => depth = depth.saturating_sub(1),
                        b'>' if depth == 0 => {
                            at += 1;
                            current.mode = Mode::Text;
                            styles.push(start, at, STYLE_KEYWORD);
                            continue;
                        }
                        _ => {}
                    }
                }
                at += width;
                current.mode = Mode::Declaration { depth, quote };
                Some(STYLE_KEYWORD)
            }
            Mode::Reference { length, kind } => {
                let next = match kind {
                    Reference::Start if byte == b'#' => Some(Reference::Numeric),
                    Reference::Start if name_start(ch) => Some(Reference::Name),
                    Reference::Numeric if byte == b'x' => Some(Reference::HexStart),
                    Reference::Numeric if byte.is_ascii_digit() => Some(Reference::Decimal),
                    Reference::HexStart if byte.is_ascii_hexdigit() => Some(Reference::Hex),
                    Reference::Decimal if byte.is_ascii_digit() => Some(kind),
                    Reference::Hex if byte.is_ascii_hexdigit() => Some(kind),
                    Reference::Name if name_char(ch) => Some(kind),
                    _ => None,
                };
                if let Some(kind) = next {
                    at += width;
                    current.mode = Mode::Reference {
                        length: length.checked_add(width as u64)?,
                        kind,
                    };
                } else {
                    if byte == b';'
                        && matches!(kind, Reference::Name | Reference::Decimal | Reference::Hex)
                    {
                        at += 1;
                        styles.push_back(
                            length.checked_add(1)?,
                            at,
                            if kind == Reference::Name {
                                STYLE_OPERATOR
                            } else {
                                STYLE_NUMBER
                            },
                        );
                    } else {
                        styles.push_back(length, at, STYLE_PLAIN);
                    }
                    current.mode = Mode::Text;
                }
                continue;
            }
        };
        if let Some(role) = role {
            styles.push(start, at, role);
        }
    }
    if final_chunk
        && at == text.len()
        && let Mode::Reference { length, .. } = current.mode
    {
        styles.push_back(length, at, STYLE_PLAIN);
        current.mode = Mode::Text;
    }
    if cancelled() {
        return None;
    }
    *state = current;
    Some(at)
}

// XML 1.0 Fifth Edition productions [4] / [4a]. Names are lexed, not validated
// for namespaces or declarations. No table or grammar is loaded from content.
fn name_start(ch: char) -> bool {
    matches!(ch, ':' | '_' | 'A'..='Z' | 'a'..='z'
        | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}'
        | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}'
        | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}'
        | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}')
}

fn name_char(ch: char) -> bool {
    name_start(ch)
        || matches!(ch, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
}

#[cfg(test)]
#[path = "syntax_xml_stream_tests.rs"]
mod stream_tests;
