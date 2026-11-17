//! XML 1.0 lexical colouring only: no validation, entity expansion or external reads.
use super::{Highlight, Poll, Spans};
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
        name: bool,
    },
    Quote {
        quote: u8,
        processing: bool,
    },
    Comment {
        declaration: Option<u8>,
    },
    Cdata,
    Declaration {
        depth: u8,
        quote: Option<u8>,
    },
}

pub(super) fn highlight<F: Fn() -> bool>(
    text: &str,
    state: &mut State,
    poll: &mut Poll<F>,
) -> Option<Highlight> {
    // The caller caps the decoded input at 64 KiB. Commit only a complete scan;
    // a cancellation or exhausted bound must leave the last checkpoint intact.
    let mut current = *state;
    let mut spans = Spans::default();
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if poll.cancelled(at) {
            return None;
        }
        let start = at;
        let role = match current.mode {
            Mode::Text => {
                if bytes[at..].starts_with(b"<!--") {
                    current.mode = Mode::Comment { declaration: None };
                    continue;
                } else if bytes[at..].starts_with(b"<![CDATA[") {
                    current.mode = Mode::Cdata;
                    continue;
                } else if bytes[at..].starts_with(b"<!") {
                    current.mode = Mode::Declaration {
                        depth: 0,
                        quote: None,
                    };
                    continue;
                } else if bytes[at] == b'<' {
                    at += 1;
                    let processing = bytes.get(at) == Some(&b'?');
                    if processing || bytes.get(at) == Some(&b'/') {
                        at += 1;
                    }
                    current.mode = Mode::Tag {
                        processing,
                        name: true,
                    };
                    Some(STYLE_OPERATOR)
                } else if bytes[at] == b'&' {
                    let (end, role) = reference(text, at, poll)?;
                    at = end;
                    role
                } else {
                    while at < bytes.len() && !matches!(bytes[at], b'<' | b'&') {
                        if poll.cancelled(at) {
                            return None;
                        }
                        at += text[at..].chars().next().unwrap().len_utf8();
                    }
                    None
                }
            }
            Mode::Tag { processing, name } => {
                let close = if processing { b"?>" } else { b"/>" };
                if bytes[at..].starts_with(close) {
                    at += 2;
                    current.mode = Mode::Text;
                    Some(STYLE_OPERATOR)
                } else if !processing && bytes[at] == b'>' {
                    at += 1;
                    current.mode = Mode::Text;
                    Some(STYLE_OPERATOR)
                } else if matches!(bytes[at], b'\'' | b'"') {
                    let quote = bytes[at];
                    at += 1;
                    current.mode = Mode::Quote { quote, processing };
                    Some(STYLE_STRING)
                } else if name_start(text[at..].chars().next().unwrap()) {
                    at = name_end(text, at, poll)?;
                    current.mode = Mode::Tag {
                        processing,
                        name: false,
                    };
                    Some(if name { STYLE_KEYWORD } else { STYLE_TYPE })
                } else {
                    let operator = matches!(bytes[at], b'=' | b'?' | b'/' | b'>');
                    at += text[at..].chars().next().unwrap().len_utf8();
                    operator.then_some(STYLE_OPERATOR)
                }
            }
            Mode::Quote { quote, processing } => {
                while at < bytes.len() {
                    if poll.cancelled(at) {
                        return None;
                    }
                    let closed = bytes[at] == quote;
                    at += text[at..].chars().next().unwrap().len_utf8();
                    if closed {
                        current.mode = Mode::Tag {
                            processing,
                            name: false,
                        };
                        break;
                    }
                }
                Some(STYLE_STRING)
            }
            Mode::Comment { declaration } => {
                let (end, closed) = delimited(text, at, b"-->", poll)?;
                at = end;
                if closed {
                    current.mode = declaration
                        .map_or(Mode::Text, |depth| Mode::Declaration { depth, quote: None });
                }
                Some(STYLE_COMMENT)
            }
            Mode::Cdata => {
                let (end, closed) = delimited(text, at, b"]]>", poll)?;
                at = end;
                if closed {
                    current.mode = Mode::Text;
                }
                Some(STYLE_STRING)
            }
            Mode::Declaration {
                mut depth,
                mut quote,
            } => {
                while at < bytes.len() {
                    if poll.cancelled(at) {
                        return None;
                    }
                    if let Some(delimiter) = quote {
                        if bytes[at] == delimiter {
                            quote = None;
                        }
                    } else if bytes[at..].starts_with(b"<!--") {
                        current.mode = Mode::Comment {
                            declaration: Some(depth),
                        };
                        break;
                    } else {
                        match bytes[at] {
                            b'\'' | b'"' => quote = Some(bytes[at]),
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
                                break;
                            }
                            _ => {}
                        }
                    }
                    at += text[at..].chars().next().unwrap().len_utf8();
                    current.mode = Mode::Declaration { depth, quote };
                }
                Some(STYLE_KEYWORD)
            }
        };
        if let Some(role) = role {
            spans.push(start, at, role);
        }
    }
    *state = current;
    Some(spans.0)
}

fn delimited<F: Fn() -> bool>(
    text: &str,
    mut at: usize,
    close: &[u8],
    poll: &mut Poll<F>,
) -> Option<(usize, bool)> {
    while at < text.len() {
        if poll.cancelled(at) {
            return None;
        }
        if text.as_bytes()[at..].starts_with(close) {
            return Some((at + close.len(), true));
        }
        at += text[at..].chars().next().unwrap().len_utf8();
    }
    Some((at, false))
}

fn name_end<F: Fn() -> bool>(text: &str, mut at: usize, poll: &mut Poll<F>) -> Option<usize> {
    for ch in text[at..].chars() {
        if poll.cancelled(at) {
            return None;
        }
        if !name_char(ch) {
            break;
        }
        at += ch.len_utf8();
    }
    Some(at)
}

fn reference<F: Fn() -> bool>(
    text: &str,
    start: usize,
    poll: &mut Poll<F>,
) -> Option<(usize, Option<u16>)> {
    let bytes = text.as_bytes();
    let mut at = start + 1;
    let numeric = bytes.get(at) == Some(&b'#');
    if numeric {
        at += 1;
        let hex = bytes.get(at) == Some(&b'x');
        if hex {
            at += 1;
        }
        let digits = at;
        while let Some(byte) = bytes.get(at) {
            if poll.cancelled(at) {
                return None;
            }
            if !(if hex {
                byte.is_ascii_hexdigit()
            } else {
                byte.is_ascii_digit()
            }) {
                break;
            }
            at += 1;
        }
        if at == digits {
            return Some((at, None));
        }
    } else if text
        .get(at..)
        .and_then(|tail| tail.chars().next())
        .is_some_and(name_start)
    {
        at = name_end(text, at, poll)?;
    } else {
        return Some((at, None));
    }
    if bytes.get(at) == Some(&b';') {
        Some((
            at + 1,
            Some(if numeric {
                STYLE_NUMBER
            } else {
                STYLE_OPERATOR
            }),
        ))
    } else {
        Some((at, None))
    }
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
