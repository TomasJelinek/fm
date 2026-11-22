//! Lua lexical state only; long bracket levels are counters, never copied delimiters.
use midnight_plugin_sdk::api::viewer::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum State {
    #[default]
    Initial,
    Normal,
    Identifier,
    Number(Number),
    CommentStart,
    LineComment,
    Quote {
        quote: u8,
        escaped: bool,
        whitespace: bool,
    },
    Opening {
        equals: u64,
        comment: bool,
    },
    Long {
        level: u64,
        equals: u64,
        closing: bool,
        comment: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Number {
    Zero,
    Integer { hex: bool },
    Fraction { hex: bool },
    Exponent,
    ExponentDigits,
}

impl State {
    pub(super) fn pending_style(&self) -> bool {
        self.pending_span().is_some()
    }

    /// Range of a consumed opener whose role still depends on following input.
    pub(super) fn pending_span(&self) -> Option<(u64, u64)> {
        match self {
            Self::Opening {
                equals,
                comment: false,
            } => Some((equals + 1, 0)),
            _ => None,
        }
    }
}

const KEYWORDS: &str = "and break do else elseif end false for function goto if in local nil not or repeat return then true until while";

pub(super) fn scan(
    text: &str,
    state: &mut State,
    styles: &mut crate::styles::Styles,
    cancelled: impl Fn() -> bool,
) -> Option<()> {
    (stream(text, state, styles, true, cancelled)? == text.len()).then_some(())
}

/// Consume a bounded decoded turn. A short ambiguous keyword or punctuation
/// suffix stays with the caller; arbitrary bracket levels use numeric state.
pub(super) fn stream(
    text: &str,
    state: &mut State,
    styles: &mut crate::styles::Styles,
    final_chunk: bool,
    cancelled: impl Fn() -> bool,
) -> Option<usize> {
    let mut poll = super::Poll::new(cancelled);
    if text.len() > 65_536 || poll.cancelled(0) {
        return None;
    }
    let mut current = *state;
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if poll.cancelled(at) {
            return None;
        }
        let start = at;
        let byte = bytes[at];
        let ch = text[at..].chars().next()?;
        let width = ch.len_utf8();
        let role = match current {
            State::Initial => {
                if byte == b'#' && bytes.len() - at == 1 && !final_chunk {
                    break;
                }
                current = if bytes[at..].starts_with(b"#!") {
                    State::LineComment
                } else {
                    State::Normal
                };
                continue;
            }
            State::Identifier => {
                if byte.is_ascii_alphanumeric() || byte == b'_' {
                    at += width;
                    STYLE_PLAIN
                } else {
                    current = State::Normal;
                    continue;
                }
            }
            State::Number(number) => {
                let next = match number {
                    Number::Zero if matches!(byte, b'x' | b'X') => {
                        Some(Number::Integer { hex: true })
                    }
                    Number::Zero | Number::Integer { hex: false } if byte.is_ascii_digit() => {
                        Some(Number::Integer { hex: false })
                    }
                    Number::Integer { hex: true } if byte.is_ascii_hexdigit() => Some(number),
                    Number::Zero | Number::Integer { .. } if byte == b'.' => {
                        if at + 1 == bytes.len() && !final_chunk {
                            break;
                        }
                        if bytes.get(at + 1) == Some(&b'.') {
                            None
                        } else {
                            Some(Number::Fraction {
                                hex: matches!(number, Number::Integer { hex: true }),
                            })
                        }
                    }
                    Number::Fraction { hex }
                        if if hex {
                            byte.is_ascii_hexdigit()
                        } else {
                            byte.is_ascii_digit()
                        } =>
                    {
                        Some(number)
                    }
                    Number::Zero
                    | Number::Integer { hex: false }
                    | Number::Fraction { hex: false }
                        if matches!(byte, b'e' | b'E') =>
                    {
                        Some(Number::Exponent)
                    }
                    Number::Integer { hex: true } | Number::Fraction { hex: true }
                        if matches!(byte, b'p' | b'P') =>
                    {
                        Some(Number::Exponent)
                    }
                    Number::Exponent if matches!(byte, b'+' | b'-') => Some(Number::ExponentDigits),
                    Number::Exponent | Number::ExponentDigits if byte.is_ascii_digit() => {
                        Some(Number::ExponentDigits)
                    }
                    _ => None,
                };
                if let Some(next) = next {
                    current = State::Number(next);
                    at += width;
                    STYLE_NUMBER
                } else {
                    current = State::Normal;
                    continue;
                }
            }
            State::CommentStart => {
                current = if byte == b'[' {
                    at += 1;
                    styles.push(start, at, STYLE_COMMENT);
                    State::Opening {
                        equals: 0,
                        comment: true,
                    }
                } else {
                    State::LineComment
                };
                continue;
            }
            State::LineComment => {
                if byte == b'\n' {
                    current = State::Normal;
                    continue;
                }
                at += width;
                STYLE_COMMENT
            }
            State::Opening { equals, comment } => {
                if byte == b'=' {
                    let equals = equals.checked_add(1)?;
                    equals.checked_add(2)?;
                    current = State::Opening { equals, comment };
                    at += 1;
                    if comment {
                        styles.push(start, at, STYLE_COMMENT);
                    }
                    continue;
                }
                if byte == b'[' {
                    at += 1;
                    if comment {
                        styles.push(start, at, STYLE_COMMENT);
                    } else {
                        styles.push_back(equals.checked_add(2)?, at, STYLE_STRING);
                    }
                    current = State::Long {
                        level: equals,
                        equals: 0,
                        closing: false,
                        comment,
                    };
                } else {
                    if !comment {
                        styles.push_back(equals.checked_add(1)?, at, STYLE_PLAIN);
                    }
                    current = if comment {
                        State::LineComment
                    } else {
                        State::Normal
                    };
                }
                continue;
            }
            State::Long {
                level,
                mut equals,
                mut closing,
                comment,
            } => {
                at += width;
                if byte == b']' && closing && equals == level {
                    current = State::Normal;
                } else {
                    if byte == b']' {
                        closing = true;
                        equals = 0;
                    } else if byte == b'=' && closing {
                        equals = equals.checked_add(1)?;
                        if equals > level {
                            closing = false;
                            equals = 0;
                        }
                    } else {
                        closing = false;
                        equals = 0;
                    }
                    current = State::Long {
                        level,
                        equals,
                        closing,
                        comment,
                    };
                }
                if comment { STYLE_COMMENT } else { STYLE_STRING }
            }
            State::Quote {
                quote,
                mut escaped,
                mut whitespace,
            } => {
                at += width;
                if whitespace && byte.is_ascii_whitespace() {
                    current = State::Quote {
                        quote,
                        escaped,
                        whitespace,
                    };
                } else {
                    whitespace = false;
                    if escaped {
                        escaped = false;
                        whitespace = byte == b'z';
                    } else if byte == b'\\' {
                        escaped = true;
                    } else if byte == quote || byte == b'\n' {
                        current = State::Normal;
                    }
                    if current != State::Normal {
                        current = State::Quote {
                            quote,
                            escaped,
                            whitespace,
                        };
                    }
                }
                STYLE_STRING
            }
            State::Normal => {
                if byte == b'-' && at + 1 == bytes.len() && !final_chunk {
                    break;
                }
                if bytes[at..].starts_with(b"--") {
                    at += 2;
                    current = State::CommentStart;
                    STYLE_COMMENT
                } else if byte == b'[' {
                    at += 1;
                    current = State::Opening {
                        equals: 0,
                        comment: false,
                    };
                    continue;
                } else if matches!(byte, b'\'' | b'"') {
                    at += 1;
                    current = State::Quote {
                        quote: byte,
                        escaped: false,
                        whitespace: false,
                    };
                    STYLE_STRING
                } else if byte.is_ascii_alphabetic() || byte == b'_' {
                    let length = bytes[at..]
                        .iter()
                        .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
                        .take(super::words::NAME_BYTES + 1)
                        .count();
                    if length > super::words::NAME_BYTES {
                        // Too long to classify; a continuation stays plain.
                        at += length;
                        current = State::Identifier;
                        STYLE_PLAIN
                    } else if at + length == bytes.len() && !final_chunk {
                        // A keyword, a call or a plain name: the next byte decides.
                        break;
                    } else {
                        at += length;
                        if KEYWORDS
                            .split_ascii_whitespace()
                            .any(|word| word == &text[start..at])
                        {
                            STYLE_KEYWORD
                        } else if bytes.get(at) == Some(&b'(') {
                            STYLE_FUNCTION
                        } else {
                            STYLE_PLAIN
                        }
                    }
                } else if byte.is_ascii_digit() {
                    at += 1;
                    current = State::Number(if byte == b'0' {
                        Number::Zero
                    } else {
                        Number::Integer { hex: false }
                    });
                    STYLE_NUMBER
                } else if byte == b'.' {
                    if at + 1 == bytes.len() && !final_chunk {
                        break;
                    }
                    at += 1;
                    if bytes.get(at).is_some_and(u8::is_ascii_digit) {
                        current = State::Number(Number::Fraction { hex: false });
                        STYLE_NUMBER
                    } else {
                        STYLE_PLAIN
                    }
                } else {
                    at += width;
                    STYLE_PLAIN
                }
            }
        };
        styles.push(start, at, role);
    }
    if final_chunk && at == text.len() {
        current = match current {
            State::Opening {
                equals,
                comment: false,
            } => {
                styles.push_back(equals.checked_add(1)?, at, STYLE_PLAIN);
                State::Normal
            }
            State::Opening { .. }
            | State::CommentStart
            | State::LineComment
            | State::Identifier
            | State::Number(_) => State::Normal,
            other => other,
        };
    }
    if poll.cancelled(at) {
        return None;
    }
    *state = current;
    Some(at)
}

#[cfg(test)]
#[path = "syntax_lua_stream_tests.rs"]
mod stream_tests;
