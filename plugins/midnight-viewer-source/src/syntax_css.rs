//! Bounded CSS lexical colouring: no cascade, value parsing or resource loading.
use midnight_plugin_sdk::api::viewer::*;

/// At-rule names whose blocks hold what their parent holds (conditional group
/// rules) followed by keyframe rules, whose blocks hold keyframe selectors.
const AT_RULES: [&[u8]; 12] = [
    b"media",
    b"supports",
    b"container",
    b"layer",
    b"scope",
    b"document",
    b"-moz-document",
    b"starting-style",
    b"keyframes",
    b"-webkit-keyframes",
    b"-moz-keyframes",
    b"-o-keyframes",
];
const CONDITIONAL: usize = 8;
const ALL_AT_RULES: u16 = (1 << AT_RULES.len()) - 1;
/// Block levels whose kind is remembered; deeper blocks hold declarations.
const TRACKED_DEPTH: u32 = 64;
const NO_URL: u8 = u8::MAX;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct State {
    mode: Mode,
    context: Context,
    at_rule: AtRule,
    /// Open blocks, saturating.
    depth: u32,
    /// Bit `n` is set when the block at depth `n + 1` holds rules; the top level does.
    rules: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Context {
    /// The start of a rule, at-rule or declaration.
    #[default]
    Statement,
    Selector,
    Attribute {
        value: bool,
    },
    /// An at-rule prelude, up to its block or semicolon.
    Prelude,
    /// A declaration value, up to its semicolon or the end of the block.
    Value,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AtRule {
    #[default]
    Other,
    Conditional,
    Keyframes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Normal,
    Comment {
        star: bool,
    },
    Quote {
        quote: u8,
        escaped: bool,
        /// The last byte was an escaped CR: an LF right after it is the rest of the same
        /// escaped line break (CRLF), not an unescaped one.
        after_cr: bool,
    },
    /// After `url(`: a quoted URL is an ordinary string.
    UrlStart,
    Url {
        escaped: bool,
    },
    /// The rest of a name whose role is already known.
    Name {
        role: u16,
    },
    AtName {
        candidates: u16,
        length: u8,
    },
    Number,
    /// After `!`, before its keyword.
    Bang,
    /// A name whose role depends on what follows it.
    Word(Word),
}

/// A consumed name, and for declarations and preludes its trailing whitespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Word {
    length: u64,
    trailing: u64,
    kind: WordKind,
    /// Matched prefix of `url`, or `NO_URL`.
    url: u8,
    /// Leading hyphens, up to two (`--custom` properties).
    dashes: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WordKind {
    /// A property name, or a nested selector when no colon follows.
    Declaration,
    /// A function name when `(` follows at once.
    Value,
    /// A function name, or a media feature when a colon follows.
    Prelude,
}

impl Word {
    fn new(kind: WordKind) -> Self {
        Self {
            length: 0,
            trailing: 0,
            kind,
            url: 0,
            dashes: 0,
        }
    }
}

impl State {
    pub(super) fn pending_style(&self) -> bool {
        self.pending_span().is_some()
    }

    /// A name's consumed length and excluded trailing whitespace, relative to the
    /// cursor, while its role still depends on following input.
    pub(super) fn pending_span(&self) -> Option<(u64, u64)> {
        match self.mode {
            Mode::Word(word) if word.length > word.trailing => Some((word.length, word.trailing)),
            _ => None,
        }
    }

    fn holds_rules(&self) -> bool {
        self.depth == 0 || (self.depth <= TRACKED_DEPTH && self.rules & bit(self.depth) != 0)
    }

    /// The kind of block that `{` opens here.
    fn opens_rules(&self) -> bool {
        self.context == Context::Prelude
            && match self.at_rule {
                AtRule::Conditional => self.holds_rules(),
                AtRule::Keyframes => true,
                AtRule::Other => false,
            }
    }

    fn open(&mut self) {
        let rules = self.opens_rules();
        self.depth = self.depth.saturating_add(1);
        if self.depth <= TRACKED_DEPTH {
            if rules {
                self.rules |= bit(self.depth);
            } else {
                self.rules &= !bit(self.depth);
            }
        }
        self.context = Context::Statement;
        self.at_rule = AtRule::Other;
    }

    fn close(&mut self) {
        if self.depth > 0 {
            if self.depth <= TRACKED_DEPTH {
                self.rules &= !bit(self.depth);
            }
            self.depth -= 1;
        }
        self.context = Context::Statement;
        self.at_rule = AtRule::Other;
    }
}

/// The mask bit of block depth `depth` (1..=64).
fn bit(depth: u32) -> u64 {
    1 << (depth - 1)
}

pub(super) fn scan(
    text: &str,
    state: &mut State,
    styles: &mut crate::styles::Styles,
    cancelled: impl Fn() -> bool,
) -> Option<()> {
    (stream(text, state, styles, true, cancelled)? == text.len()).then_some(())
}

/// Consume a bounded decoded turn. Names of any length keep counters, never text;
/// at most a few bytes of an ambiguous delimiter, number or colour stay with the caller.
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
        match current.mode {
            Mode::Comment { star } => {
                at += width;
                styles.push(start, at, STYLE_COMMENT);
                current.mode = if star && byte == b'/' {
                    Mode::Normal
                } else {
                    Mode::Comment { star: byte == b'*' }
                };
            }
            Mode::Quote {
                quote,
                escaped,
                after_cr,
            } => {
                let continued = after_cr && byte == b'\n';
                if !escaped && !continued && matches!(byte, b'\n' | b'\r' | b'\x0c') {
                    // An unescaped line break ends a (bad) string before itself.
                    current.mode = Mode::Normal;
                    continue;
                }
                at += width;
                styles.push(start, at, STYLE_STRING);
                current.mode = if escaped {
                    Mode::Quote {
                        quote,
                        escaped: false,
                        after_cr: byte == b'\r',
                    }
                } else if continued {
                    Mode::Quote {
                        quote,
                        escaped: false,
                        after_cr: false,
                    }
                } else if byte == b'\\' {
                    Mode::Quote {
                        quote,
                        escaped: true,
                        after_cr: false,
                    }
                } else if byte == quote {
                    Mode::Normal
                } else {
                    Mode::Quote {
                        quote,
                        escaped: false,
                        after_cr: false,
                    }
                };
            }
            Mode::UrlStart => {
                if ch.is_whitespace() {
                    at += width;
                } else if matches!(byte, b'"' | b'\'' | b')') {
                    current.mode = Mode::Normal;
                } else {
                    current.mode = Mode::Url { escaped: false };
                }
            }
            Mode::Url { escaped } => {
                if !escaped && byte == b')' {
                    at += 1;
                    current.mode = Mode::Normal;
                    continue;
                }
                at += width;
                styles.push(start, at, STYLE_STRING);
                current.mode = Mode::Url {
                    escaped: !escaped && byte == b'\\',
                };
            }
            Mode::Name { role } => match name_part(bytes, at, final_chunk) {
                None => break,
                Some(false) => current.mode = Mode::Normal,
                Some(true) => {
                    at += part_width(text, at, width)?;
                    styles.push(start, at, role);
                }
            },
            Mode::AtName { candidates, length } => {
                if name_char(byte) {
                    at += width;
                    styles.push(start, at, STYLE_KEYWORD);
                    current.mode = Mode::AtName {
                        candidates: narrow(candidates, length, byte),
                        length: length.saturating_add(1),
                    };
                } else {
                    current.at_rule = at_rule(candidates, length);
                    current.mode = Mode::Normal;
                }
            }
            Mode::Number => {
                if name_char(byte) {
                    at += width;
                    styles.push(start, at, STYLE_NUMBER);
                } else if byte == b'%' {
                    at += 1;
                    styles.push(start, at, STYLE_NUMBER);
                    current.mode = Mode::Normal;
                } else if byte == b'.' {
                    match bytes.get(at + 1) {
                        None if !final_chunk => break,
                        Some(next) if next.is_ascii_digit() => {
                            at += 1;
                            styles.push(start, at, STYLE_NUMBER);
                        }
                        _ => current.mode = Mode::Normal,
                    }
                } else {
                    current.mode = Mode::Normal;
                }
            }
            Mode::Bang => {
                if ch.is_whitespace() {
                    at += width;
                    continue;
                }
                current.mode = match ident_start(bytes, at, final_chunk) {
                    None => break,
                    Some(true) => Mode::Name {
                        role: STYLE_KEYWORD,
                    },
                    Some(false) => Mode::Normal,
                };
            }
            Mode::Word(mut word) => {
                if word.trailing == 0 {
                    match name_part(bytes, at, final_chunk) {
                        None => break,
                        Some(false) => {}
                        Some(true) => {
                            let consumed = part_width(text, at, width)?;
                            word.url = if byte == b'\\' {
                                NO_URL
                            } else {
                                match word.url {
                                    matched @ 0..=2
                                        if byte.eq_ignore_ascii_case(&b"url"[matched as usize]) =>
                                    {
                                        matched + 1
                                    }
                                    _ => NO_URL,
                                }
                            };
                            if byte == b'-'
                                && word.length == u64::from(word.dashes)
                                && word.dashes < 2
                            {
                                word.dashes += 1;
                            }
                            word.length = word.length.checked_add(consumed as u64)?;
                            at += consumed;
                            current.mode = Mode::Word(word);
                            continue;
                        }
                    }
                }
                if word.kind != WordKind::Value && ch.is_whitespace() {
                    word.trailing = word.trailing.checked_add(width as u64)?;
                    word.length = word.length.checked_add(width as u64)?;
                    at += width;
                    current.mode = Mode::Word(word);
                    continue;
                }
                let call = word.kind != WordKind::Declaration && byte == b'(' && word.trailing == 0;
                let role = match word.kind {
                    WordKind::Declaration if byte == b':' => {
                        current.context = Context::Value;
                        STYLE_TYPE
                    }
                    WordKind::Declaration if selector_continues(byte) => {
                        current.context = Context::Selector;
                        STYLE_FUNCTION
                    }
                    WordKind::Declaration => STYLE_PLAIN,
                    _ if call => STYLE_FUNCTION,
                    WordKind::Prelude if byte == b':' => STYLE_TYPE,
                    _ if word.dashes == 2 => STYLE_TYPE,
                    _ => STYLE_PLAIN,
                };
                styles.push_pending(word.length, word.trailing, at, role);
                current.mode = Mode::Normal;
                if word.kind == WordKind::Declaration && byte == b':' {
                    at += 1;
                } else if call {
                    at += 1;
                    if word.url == 3 {
                        current.mode = Mode::UrlStart;
                    }
                }
            }
            Mode::Normal => {
                if byte == b'/' {
                    match bytes.get(at + 1) {
                        None if !final_chunk => break,
                        Some(b'*') => {
                            at += 2;
                            styles.push(start, at, STYLE_COMMENT);
                            current.mode = Mode::Comment { star: false };
                            continue;
                        }
                        _ => {}
                    }
                }
                match byte {
                    b'"' | b'\'' => {
                        at += 1;
                        styles.push(start, at, STYLE_STRING);
                        current.mode = Mode::Quote {
                            quote: byte,
                            escaped: false,
                            after_cr: false,
                        };
                        continue;
                    }
                    b'{' => {
                        at += 1;
                        current.open();
                        continue;
                    }
                    b'}' => {
                        at += 1;
                        current.close();
                        continue;
                    }
                    b';' => {
                        at += 1;
                        current.context = Context::Statement;
                        current.at_rule = AtRule::Other;
                        continue;
                    }
                    _ => {}
                }
                match current.context {
                    Context::Statement => {
                        if ch.is_whitespace() {
                            at += width;
                        } else if byte == b'@' {
                            at += 1;
                            styles.push(start, at, STYLE_KEYWORD);
                            current.mode = Mode::AtName {
                                candidates: ALL_AT_RULES,
                                length: 0,
                            };
                            current.context = Context::Prelude;
                            current.at_rule = AtRule::Other;
                        } else if current.holds_rules() {
                            current.context = Context::Selector;
                        } else {
                            match ident_start(bytes, at, final_chunk) {
                                None => break,
                                Some(true) => {
                                    current.mode = Mode::Word(Word::new(WordKind::Declaration));
                                }
                                Some(false) if selector_continues(byte) => {
                                    current.context = Context::Selector;
                                }
                                Some(false) => at += width,
                            }
                        }
                    }
                    Context::Selector => {
                        if byte == b'[' {
                            at += 1;
                            current.context = Context::Attribute { value: false };
                        } else if matches!(byte, b'*' | b'&' | b':') {
                            at += 1;
                            styles.push(start, at, STYLE_FUNCTION);
                        } else if byte == b'#' {
                            match bytes.get(at + 1) {
                                None if !final_chunk => break,
                                Some(next) if name_char(*next) => {
                                    at += 1;
                                    styles.push(start, at, STYLE_FUNCTION);
                                    current.mode = Mode::Name {
                                        role: STYLE_FUNCTION,
                                    };
                                }
                                _ => at += 1,
                            }
                        } else if byte == b'.' {
                            match (
                                ident_start(bytes, at + 1, final_chunk),
                                number_start(bytes, at, final_chunk),
                            ) {
                                (None, _) | (_, None) => break,
                                (Some(true), _) => {
                                    at += 1;
                                    styles.push(start, at, STYLE_FUNCTION);
                                    current.mode = Mode::Name {
                                        role: STYLE_FUNCTION,
                                    };
                                }
                                (_, Some(0)) => at += 1,
                                (_, Some(length)) => {
                                    at += length;
                                    styles.push(start, at, STYLE_NUMBER);
                                    current.mode = Mode::Number;
                                }
                            }
                        } else {
                            match ident_start(bytes, at, final_chunk) {
                                None => break,
                                Some(true) => {
                                    current.mode = Mode::Name {
                                        role: STYLE_FUNCTION,
                                    }
                                }
                                Some(false) if byte.is_ascii_digit() => {
                                    at += 1;
                                    styles.push(start, at, STYLE_NUMBER);
                                    current.mode = Mode::Number;
                                }
                                Some(false) => at += width,
                            }
                        }
                    }
                    Context::Attribute { value } => {
                        if byte == b']' {
                            at += 1;
                            current.context = Context::Selector;
                        } else if byte == b'=' {
                            at += 1;
                            current.context = Context::Attribute { value: true };
                        } else {
                            match ident_start(bytes, at, final_chunk) {
                                None => break,
                                Some(true) => {
                                    current.mode = Mode::Name {
                                        role: if value { STYLE_STRING } else { STYLE_FUNCTION },
                                    }
                                }
                                Some(false) => at += width,
                            }
                        }
                    }
                    Context::Prelude | Context::Value => {
                        let value = current.context == Context::Value;
                        if ch.is_whitespace() {
                            at += width;
                            continue;
                        }
                        match ident_start(bytes, at, final_chunk) {
                            None => break,
                            Some(true) => {
                                current.mode = Mode::Word(Word::new(if value {
                                    WordKind::Value
                                } else {
                                    WordKind::Prelude
                                }));
                                continue;
                            }
                            Some(false) => {}
                        }
                        match number_start(bytes, at, final_chunk) {
                            None => break,
                            Some(0) => {}
                            Some(length) => {
                                at += length;
                                styles.push(start, at, STYLE_NUMBER);
                                current.mode = Mode::Number;
                                continue;
                            }
                        }
                        if value && byte == b'#' {
                            match colour(bytes, at, final_chunk) {
                                None => break,
                                Some(Some(length)) => {
                                    at += length;
                                    styles.push(start, at, STYLE_NUMBER);
                                }
                                Some(None) => {
                                    at += 1;
                                    current.mode = Mode::Name { role: STYLE_PLAIN };
                                }
                            }
                        } else if value && byte == b'!' {
                            at += 1;
                            styles.push(start, at, STYLE_KEYWORD);
                            current.mode = Mode::Bang;
                        } else {
                            at += width;
                        }
                    }
                }
            }
        }
    }
    if final_chunk && at == text.len() {
        match current.mode {
            Mode::Word(word) => {
                // Without a following colon a declaration name is still a property.
                let role = match word.kind {
                    WordKind::Declaration => STYLE_TYPE,
                    _ if word.dashes == 2 => STYLE_TYPE,
                    _ => STYLE_PLAIN,
                };
                styles.push_pending(word.length, word.trailing, at, role);
                current.mode = Mode::Normal;
            }
            Mode::AtName { candidates, length } => {
                current.at_rule = at_rule(candidates, length);
                current.mode = Mode::Normal;
            }
            Mode::Name { .. } | Mode::Number | Mode::Bang | Mode::UrlStart => {
                current.mode = Mode::Normal;
            }
            Mode::Normal | Mode::Comment { .. } | Mode::Quote { .. } | Mode::Url { .. } => {}
        }
    }
    if poll.cancelled(at) {
        return None;
    }
    *state = current;
    Some(at)
}

fn name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

fn name_char(byte: u8) -> bool {
    name_start(byte) || byte.is_ascii_digit() || byte == b'-'
}

/// Characters after a declaration-position name that make it a nested selector.
fn selector_continues(byte: u8) -> bool {
    name_char(byte)
        || matches!(
            byte,
            b'\\' | b'{' | b',' | b'>' | b'+' | b'~' | b'.' | b'#' | b'[' | b'&' | b'*' | b'|'
        )
}

/// The byte at `offset` after `at`: `Some(None)` at the end of the final chunk,
/// `None` when the next chunk must decide.
fn peek(bytes: &[u8], at: usize, offset: usize, final_chunk: bool) -> Option<Option<u8>> {
    match bytes.get(at + offset) {
        Some(byte) => Some(Some(*byte)),
        None if final_chunk => Some(None),
        None => None,
    }
}

/// A backslash escape: any character but a line break.
fn escape(next: Option<u8>) -> bool {
    next.is_some_and(|byte| !matches!(byte, b'\n' | b'\r' | b'\x0c'))
}

/// Whether an identifier starts at `at`. `None`: more input is needed.
fn ident_start(bytes: &[u8], at: usize, final_chunk: bool) -> Option<bool> {
    let Some(first) = peek(bytes, at, 0, final_chunk)? else {
        return Some(false);
    };
    Some(match first {
        b'-' => match peek(bytes, at, 1, final_chunk)? {
            Some(b'-') => true,
            Some(b'\\') => escape(peek(bytes, at, 2, final_chunk)?),
            Some(next) => name_start(next),
            None => false,
        },
        b'\\' => escape(peek(bytes, at, 1, final_chunk)?),
        first => name_start(first),
    })
}

/// Whether a name character or escape continues a name at `at`.
/// `None`: more input is needed.
fn name_part(bytes: &[u8], at: usize, final_chunk: bool) -> Option<bool> {
    let byte = *bytes.get(at)?;
    if byte != b'\\' {
        return Some(name_char(byte));
    }
    Some(escape(peek(bytes, at, 1, final_chunk)?))
}

/// UTF-8 width of the name part at `at`: a character of `width` bytes, or an
/// escape's backslash with the whole following character.
fn part_width(text: &str, at: usize, width: usize) -> Option<usize> {
    if text.as_bytes().get(at) != Some(&b'\\') {
        return Some(width);
    }
    Some(1 + text.get(at + 1..)?.chars().next()?.len_utf8())
}

/// Bytes that begin a number at `at`, or 0. `None`: more input is needed.
fn number_start(bytes: &[u8], at: usize, final_chunk: bool) -> Option<usize> {
    let digit = |offset| {
        Some(peek(bytes, at, offset, final_chunk)?.is_some_and(|byte| byte.is_ascii_digit()))
    };
    Some(match peek(bytes, at, 0, final_chunk)? {
        Some(byte) if byte.is_ascii_digit() => 1,
        Some(b'.') if digit(1)? => 2,
        Some(b'+' | b'-') => match peek(bytes, at, 1, final_chunk)? {
            Some(byte) if byte.is_ascii_digit() => 2,
            Some(b'.') if digit(2)? => 3,
            _ => 0,
        },
        _ => 0,
    })
}

/// A hash in a value: `Some(Some(bytes))` for a 3, 4, 6 or 8 digit hexadecimal
/// colour, `Some(None)` for any other hash. `None`: more input is needed.
fn colour(bytes: &[u8], at: usize, final_chunk: bool) -> Option<Option<usize>> {
    let mut digits = 0;
    loop {
        match peek(bytes, at, 1 + digits, final_chunk)? {
            Some(byte) if name_char(byte) || byte == b'\\' => {
                if digits == 8 || !byte.is_ascii_hexdigit() {
                    return Some(None);
                }
                digits += 1;
            }
            _ => break,
        }
    }
    Some(matches!(digits, 3 | 4 | 6 | 8).then_some(1 + digits))
}

fn narrow(candidates: u16, length: u8, byte: u8) -> u16 {
    let mut remaining = candidates;
    for (index, name) in AT_RULES.iter().enumerate() {
        if name.get(usize::from(length)) != Some(&byte.to_ascii_lowercase()) {
            remaining &= !(1 << index);
        }
    }
    remaining
}

fn at_rule(candidates: u16, length: u8) -> AtRule {
    match AT_RULES
        .iter()
        .enumerate()
        .find(|(index, name)| candidates & (1 << index) != 0 && name.len() == usize::from(length))
    {
        Some((index, _)) if index < CONDITIONAL => AtRule::Conditional,
        Some(_) => AtRule::Keyframes,
        None => AtRule::Other,
    }
}

#[cfg(test)]
#[path = "syntax_css_stream_tests.rs"]
mod stream_tests;
