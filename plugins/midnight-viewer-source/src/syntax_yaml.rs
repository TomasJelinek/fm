//! Bounded YAML lexical colouring. No deserialization, tag resolution or expansion.
use midnight_plugin_sdk::api::viewer::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct State {
    mode: Mode,
    column: u64,
    indentation: u64,
    parent: u64,
    flow: u32,
    space: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Normal,
    Plain(Token),
    Quote {
        length: u64,
        quote: u8,
        escaped: bool,
        single_end: bool,
    },
    AfterQuote {
        length: u64,
        trailing: u64,
    },
    Comment {
        carriage_returns: u64,
    },
    Header {
        length: u64,
        trailing: u64,
        indicator: u8,
        chomp: bool,
        gap: bool,
    },
    HeaderComment {
        indent: u64,
        carriage_returns: u64,
    },
    Block {
        indent: u64,
        body: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Token {
    length: u64,
    trailing: u64,
    words: u16,
    number: u8,
}

const WORDS: [&str; 10] = [
    "true", "True", "TRUE", "false", "False", "FALSE", "null", "Null", "NULL", "~",
];
const INVALID_NUMBER: u8 = u8::MAX;

impl Token {
    fn new() -> Self {
        Self {
            length: 0,
            trailing: 0,
            words: (1 << WORDS.len()) - 1,
            number: 0,
        }
    }

    fn plain(length: u64, trailing: u64) -> Self {
        Self {
            length,
            trailing,
            words: 0,
            number: INVALID_NUMBER,
        }
    }

    fn consume(&mut self, ch: char) -> Option<()> {
        let width = ch.len_utf8() as u64;
        if ch.is_whitespace() {
            self.trailing = self.trailing.checked_add(width)?;
        } else {
            if self.trailing != 0 {
                self.words = 0;
                self.number = INVALID_NUMBER;
            }
            self.trailing = 0;
            if self.words != 0 {
                for (index, word) in WORDS.iter().enumerate() {
                    if self.words & (1 << index) != 0
                        && (!ch.is_ascii()
                            || usize::try_from(self.length)
                                .ok()
                                .and_then(|at| word.as_bytes().get(at))
                                .copied()
                                != Some(ch as u8))
                    {
                        self.words &= !(1 << index);
                    }
                }
            }
            self.number = number_next(self.number, ch);
        }
        self.length = self.length.checked_add(width)?;
        Some(())
    }

    fn role(self) -> u16 {
        let length = self.length - self.trailing;
        if WORDS
            .iter()
            .enumerate()
            .any(|(index, word)| self.words & (1 << index) != 0 && word.len() as u64 == length)
        {
            STYLE_KEYWORD
        } else if matches!(
            self.number,
            2 | 3 | 5 | 7 | 10 | 17 | 18 | 19 | 25 | 26 | 27
        ) {
            STYLE_NUMBER
        } else {
            STYLE_STRING
        }
    }
}

impl State {
    pub(super) fn pending_style(&self) -> bool {
        self.pending_span().is_some()
    }

    /// A candidate's consumed length and excluded whitespace, relative to the
    /// current cursor. Recovery may need lookahead before promising exact styles.
    pub(super) fn pending_span(&self) -> Option<(u64, u64)> {
        let span = match self.mode {
            Mode::Plain(token) => (token.length, token.trailing),
            Mode::Quote { length, .. } => (length, 0),
            Mode::AfterQuote { length, trailing } => (length.checked_add(trailing)?, trailing),
            Mode::Header {
                length, trailing, ..
            } => (length, trailing),
            Mode::Block { body: false, .. } => (self.column, 0),
            Mode::Comment { carriage_returns }
            | Mode::HeaderComment {
                carriage_returns, ..
            } => (carriage_returns, 0),
            _ => return None,
        };
        (span.0 != 0).then_some(span)
    }

    fn consume(&mut self, ch: char) -> Option<()> {
        if ch == '\n' {
            self.column = 0;
            self.indentation = 0;
        } else {
            if ch == ' ' && self.column == self.indentation {
                self.indentation = self.indentation.checked_add(1)?;
            }
            self.column = self.column.checked_add(ch.len_utf8() as u64)?;
        }
        self.space = ch.is_ascii_whitespace();
        Some(())
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

/// Consume arbitrary decoded chunks using counters and a numeric-token DFA.
/// Neither a line nor an arbitrarily long key/scalar is copied or retained.
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
        let byte = bytes[at];
        let ch = text[at..].chars().next()?;
        let width = ch.len_utf8();
        match current.mode {
            Mode::Plain(mut token) => {
                if byte == b':' && at + 1 == bytes.len() && !final_chunk {
                    break;
                }
                let key = byte == b':' && separated(bytes, at);
                if byte == b'\n'
                    || key
                    || (current.flow > 0 && matches!(byte, b',' | b'[' | b']' | b'{' | b'}'))
                    || (byte == b'#' && current.space)
                {
                    if key {
                        current.parent = current.column.checked_sub(token.length)?;
                    }
                    styles.push_pending(
                        token.length,
                        token.trailing,
                        at,
                        if key { STYLE_TYPE } else { token.role() },
                    );
                    current.mode = Mode::Normal;
                    continue;
                }
                token.consume(ch)?;
                current.consume(ch)?;
                at += width;
                current.mode = Mode::Plain(token);
            }
            Mode::Quote {
                mut length,
                quote,
                mut escaped,
                mut single_end,
            } => {
                if single_end && byte != quote {
                    current.mode = Mode::AfterQuote {
                        length,
                        trailing: 0,
                    };
                    continue;
                }
                length = length.checked_add(width as u64)?;
                current.consume(ch)?;
                at += width;
                if quote == b'\'' {
                    if byte == quote {
                        single_end = !single_end;
                    }
                } else if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == quote {
                    current.mode = Mode::AfterQuote {
                        length,
                        trailing: 0,
                    };
                    continue;
                }
                if byte == b'\n' {
                    styles.push_back(length, at, STYLE_STRING);
                    length = 0;
                    current.parent = 0;
                }
                current.mode = Mode::Quote {
                    length,
                    quote,
                    escaped,
                    single_end,
                };
            }
            Mode::AfterQuote {
                length,
                mut trailing,
            } => {
                if byte != b'\n' && byte.is_ascii_whitespace() {
                    trailing = trailing.checked_add(width as u64)?;
                    current.consume(ch)?;
                    at += width;
                    current.mode = Mode::AfterQuote { length, trailing };
                    continue;
                }
                let consumed = length.checked_add(trailing)?;
                let key = byte == b':';
                if key {
                    current.parent = current.column.checked_sub(consumed)?;
                }
                styles.push_pending(
                    consumed,
                    trailing,
                    at,
                    if key { STYLE_TYPE } else { STYLE_STRING },
                );
                current.mode = Mode::Normal;
            }
            Mode::Comment {
                mut carriage_returns,
            } => {
                if byte == b'\n' {
                    if carriage_returns != 0 {
                        styles.push_back(carriage_returns, at, STYLE_PLAIN);
                    }
                    current.consume(ch)?;
                    at += width;
                    current.parent = 0;
                    current.mode = Mode::Normal;
                } else if byte == b'\r' {
                    carriage_returns = carriage_returns.checked_add(1)?;
                    current.consume(ch)?;
                    at += 1;
                    current.mode = Mode::Comment { carriage_returns };
                } else {
                    if carriage_returns != 0 {
                        styles.push_back(carriage_returns, at, STYLE_COMMENT);
                    }
                    current.consume(ch)?;
                    at += width;
                    styles.push(start, at, STYLE_COMMENT);
                    current.mode = Mode::Comment {
                        carriage_returns: 0,
                    };
                }
            }
            Mode::Header {
                mut length,
                mut trailing,
                mut indicator,
                mut chomp,
                mut gap,
            } => {
                if !gap && matches!(byte, b'1'..=b'9') && indicator == 0 {
                    indicator = byte - b'0';
                } else if !gap && matches!(byte, b'+' | b'-') && !chomp {
                    chomp = true;
                } else if byte == b'\n' || (gap && byte == b'#') {
                    styles.push_pending(length, trailing, at, STYLE_OPERATOR);
                    let indent = if indicator == 0 {
                        0
                    } else {
                        current.parent.checked_add(u64::from(indicator))?
                    };
                    current.consume(ch)?;
                    at += width;
                    if byte == b'\n' {
                        current.mode = Mode::Block {
                            indent,
                            body: false,
                        };
                    } else {
                        styles.push(start, at, STYLE_COMMENT);
                        current.mode = Mode::HeaderComment {
                            indent,
                            carriage_returns: 0,
                        };
                    }
                    continue;
                } else if byte.is_ascii_whitespace() {
                    gap = true;
                    trailing = trailing.checked_add(width as u64)?;
                } else {
                    current.mode = Mode::Plain(Token::plain(length, trailing));
                    continue;
                }
                length = length.checked_add(width as u64)?;
                current.consume(ch)?;
                at += width;
                current.mode = Mode::Header {
                    length,
                    trailing,
                    indicator,
                    chomp,
                    gap,
                };
            }
            Mode::HeaderComment {
                indent,
                mut carriage_returns,
            } => {
                if byte == b'\n' {
                    if carriage_returns != 0 {
                        styles.push_back(carriage_returns, at, STYLE_PLAIN);
                    }
                    current.consume(ch)?;
                    at += width;
                    current.mode = Mode::Block {
                        indent,
                        body: false,
                    };
                } else if byte == b'\r' {
                    carriage_returns = carriage_returns.checked_add(1)?;
                    current.consume(ch)?;
                    at += 1;
                    current.mode = Mode::HeaderComment {
                        indent,
                        carriage_returns,
                    };
                } else {
                    if carriage_returns != 0 {
                        styles.push_back(carriage_returns, at, STYLE_COMMENT);
                    }
                    current.consume(ch)?;
                    at += width;
                    styles.push(start, at, STYLE_COMMENT);
                    current.mode = Mode::HeaderComment {
                        indent,
                        carriage_returns: 0,
                    };
                }
            }
            Mode::Block { mut indent, body } => {
                if body {
                    current.consume(ch)?;
                    at += width;
                    styles.push(start, at, STYLE_STRING);
                    current.mode = Mode::Block {
                        indent,
                        body: byte != b'\n',
                    };
                } else if ch.is_whitespace() {
                    let length = current.column.checked_add(width as u64)?;
                    current.consume(ch)?;
                    at += width;
                    if byte == b'\n' {
                        styles.push_back(length, at, STYLE_STRING);
                    }
                } else {
                    if indent == 0 && current.indentation > current.parent {
                        indent = current.indentation;
                    }
                    if indent != 0 && current.indentation >= indent {
                        let length = current.column.checked_add(width as u64)?;
                        current.consume(ch)?;
                        at += width;
                        styles.push_back(length, at, STYLE_STRING);
                        current.mode = Mode::Block { indent, body: true };
                    } else {
                        styles.push_back(current.column, at, STYLE_PLAIN);
                        current.parent = current.indentation;
                        current.mode = Mode::Normal;
                    }
                }
            }
            Mode::Normal => {
                if ch.is_ascii_whitespace() {
                    current.consume(ch)?;
                    at += width;
                    if current.column == current.indentation {
                        current.parent = current.indentation;
                    }
                } else if byte == b'#' && (current.column == 0 || current.space) {
                    current.consume(ch)?;
                    at += 1;
                    styles.push(start, at, STYLE_COMMENT);
                    current.mode = Mode::Comment {
                        carriage_returns: 0,
                    };
                } else if matches!(byte, b'\'' | b'"') {
                    current.consume(ch)?;
                    at += 1;
                    current.mode = Mode::Quote {
                        length: 1,
                        quote: byte,
                        escaped: false,
                        single_end: false,
                    };
                } else if matches!(byte, b'[' | b'{') {
                    current.flow = current.flow.checked_add(1)?;
                    current.consume(ch)?;
                    at += 1;
                    styles.push(start, at, STYLE_OPERATOR);
                } else if matches!(byte, b']' | b'}') {
                    current.flow = current.flow.saturating_sub(1);
                    current.consume(ch)?;
                    at += 1;
                    styles.push(start, at, STYLE_OPERATOR);
                } else if byte == b',' && current.flow > 0 {
                    current.consume(ch)?;
                    at += 1;
                    styles.push(start, at, STYLE_OPERATOR);
                } else if matches!(byte, b':' | b'-' | b'?') {
                    if at + 1 == bytes.len() && !final_chunk {
                        break;
                    }
                    if separated(bytes, at) {
                        current.consume(ch)?;
                        at += 1;
                        styles.push(start, at, STYLE_OPERATOR);
                    } else {
                        current.mode = Mode::Plain(Token::new());
                    }
                } else if current.flow == 0 && matches!(byte, b'|' | b'>') {
                    current.consume(ch)?;
                    at += 1;
                    current.mode = Mode::Header {
                        length: 1,
                        trailing: 0,
                        indicator: 0,
                        chomp: false,
                        gap: false,
                    };
                } else {
                    current.mode = Mode::Plain(Token::new());
                }
            }
        }
    }
    if final_chunk && at == text.len() {
        match current.mode {
            Mode::Plain(token) => {
                styles.push_pending(token.length, token.trailing, at, token.role());
                current.mode = Mode::Normal;
            }
            Mode::Quote {
                length,
                quote,
                escaped,
                single_end,
            } => {
                styles.push_back(length, at, STYLE_STRING);
                current.mode = if single_end {
                    Mode::Normal
                } else {
                    Mode::Quote {
                        length: 0,
                        quote,
                        escaped,
                        single_end: false,
                    }
                };
            }
            Mode::AfterQuote { length, trailing } => {
                styles.push_pending(length.checked_add(trailing)?, trailing, at, STYLE_STRING);
                current.mode = Mode::Normal;
            }
            Mode::Header {
                length,
                trailing,
                indicator,
                ..
            } => {
                styles.push_pending(length, trailing, at, STYLE_OPERATOR);
                let indent = if indicator == 0 {
                    0
                } else {
                    current.parent.checked_add(u64::from(indicator))?
                };
                current.mode = Mode::Block {
                    indent,
                    body: false,
                };
            }
            Mode::HeaderComment {
                indent,
                carriage_returns,
            } => {
                styles.push_back(carriage_returns, at, STYLE_PLAIN);
                current.mode = Mode::Block {
                    indent,
                    body: false,
                };
            }
            Mode::Comment { carriage_returns } => {
                styles.push_back(carriage_returns, at, STYLE_PLAIN);
                current.mode = Mode::Normal;
            }
            Mode::Block { indent, body } => {
                if !body {
                    styles.push_back(current.column, at, STYLE_STRING);
                }
                current.mode = Mode::Block {
                    indent,
                    body: false,
                };
            }
            Mode::Normal => {}
        }
        current.column = 0;
        current.indentation = 0;
        current.space = false;
        if !matches!(current.mode, Mode::Block { .. }) {
            current.parent = 0;
        }
    }
    if cancelled() {
        return None;
    }
    *state = current;
    Some(at)
}

fn separated(bytes: &[u8], at: usize) -> bool {
    bytes
        .get(at + 1)
        .is_none_or(|b| b.is_ascii_whitespace() || matches!(b, b'[' | b']' | b'{' | b'}' | b','))
}

// The existing lexical profile accepts the decimal/hexadecimal regex grammar
// and the YAML infinity/NaN spellings. The DFA carries no token text.
fn number_next(state: u8, ch: char) -> u8 {
    let byte = if ch.is_ascii() {
        ch as u8
    } else {
        return INVALID_NUMBER;
    };
    match (state, byte) {
        (0, b'+' | b'-') => 1,
        (0 | 1, b'0') => 2,
        (0 | 1, b'1'..=b'9') => 3,
        (0 | 1, b'.') => 11,
        (2, b'x' | b'X') => 4,
        (2 | 3, b'0'..=b'9' | b'_') => 3,
        (2 | 3, b'.') => 6,
        (2 | 3 | 7, b'e' | b'E') => 8,
        (4 | 5, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' | b'_') => 5,
        (6 | 7, b'0'..=b'9' | b'_') => 7,
        (8, b'+' | b'-') => 9,
        (8..=10, b'0'..=b'9' | b'_') => 10,
        (11, b'i') => 12,
        (11, b'I') => 13,
        (12, b'n') => 14,
        (13, b'n') => 15,
        (13, b'N') => 16,
        (14, b'f') => 17,
        (15, b'f') => 18,
        (16, b'F') => 19,
        (11, b'n') => 20,
        (11, b'N') => 21,
        (20, b'a') => 22,
        (21, b'a') => 23,
        (21, b'A') => 24,
        (22, b'n') => 25,
        (23, b'N') => 26,
        (24, b'N') => 27,
        _ => INVALID_NUMBER,
    }
}

#[cfg(test)]
#[path = "syntax_yaml_stream_tests.rs"]
mod stream_tests;
