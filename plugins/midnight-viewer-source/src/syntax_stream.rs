//! Incremental common profiles. Short ambiguous suffixes belong to the caller.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Number {
    Integer,
    Fraction,
    Exponent,
    Hex,
}

pub(super) fn stream(
    text: &str,
    language: Language,
    state: &mut State,
    styles: &mut Styles,
    final_chunk: bool,
    cancelled: impl Fn() -> bool,
) -> Option<usize> {
    let mut current = *state;
    let mut poll = Poll::new(cancelled);
    let bytes = text.as_bytes();
    let mut at = 0;
    'scan: while at < bytes.len() {
        if poll.cancelled(at) {
            return None;
        }
        let start = at;
        let role = match current.mode {
            Mode::Identifier => {
                while bytes
                    .get(at)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                {
                    if poll.cancelled(at) {
                        return None;
                    }
                    at += 1;
                }
                if at < bytes.len() || final_chunk {
                    current.mode = Mode::Normal;
                }
                if at == start {
                    continue;
                }
                STYLE_PLAIN
            }
            Mode::Number(number) => {
                let (end, number, done) = number.advance(bytes, at, final_chunk, &mut poll)?;
                at = end;
                current.mode = if done {
                    Mode::Normal
                } else {
                    Mode::Number(number)
                };
                if !done && at < bytes.len() {
                    styles.push(start, at, STYLE_NUMBER);
                    break 'scan;
                }
                if at == start {
                    continue;
                }
                STYLE_NUMBER
            }
            Mode::LineComment => {
                while at < bytes.len() && bytes[at] != b'\n' {
                    if poll.cancelled(at) {
                        return None;
                    }
                    at += text[at..].chars().next()?.len_utf8();
                }
                if at < bytes.len() {
                    current.mode = Mode::Normal;
                }
                if at == start {
                    continue;
                }
                STYLE_COMMENT
            }
            Mode::CppRaw(raw) => {
                let (next, closed) = raw.consume(text, at, final_chunk, &mut poll)?;
                at = next;
                if closed {
                    current.mode = Mode::Normal;
                }
                if !closed && at < bytes.len() {
                    styles.push(start, at, STYLE_STRING);
                    break 'scan;
                }
                STYLE_STRING
            }
            Mode::Xml(_) | Mode::Lua(_) | Mode::Yaml(_) | Mode::Usda(_) => {
                current.mode = Mode::Normal;
                continue;
            }
            Mode::Block(mut depth) => {
                while at < bytes.len() {
                    if poll.cancelled(at) {
                        return None;
                    }
                    if !final_chunk
                        && at + 1 == bytes.len()
                        && (bytes[at] == b'*' || (language == Language::Rust && bytes[at] == b'/'))
                    {
                        current.mode = Mode::Block(depth);
                        styles.push(start, at, STYLE_COMMENT);
                        break 'scan;
                    }
                    if bytes[at..].starts_with(b"*/") {
                        at += 2;
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    } else if language == Language::Rust && bytes[at..].starts_with(b"/*") {
                        if depth == 16 {
                            return None;
                        }
                        depth += 1;
                        at += 2;
                    } else {
                        at += text[at..].chars().next()?.len_utf8();
                    }
                }
                current.mode = if depth == 0 {
                    Mode::Normal
                } else {
                    Mode::Block(depth)
                };
                STYLE_COMMENT
            }
            Mode::Quote(mut quote) => {
                let mut closed = false;
                while at < bytes.len() {
                    if poll.cancelled(at) {
                        return None;
                    }
                    if !quote.escaped
                        && !final_chunk
                        && bytes[at] == quote.quote
                        && ((bytes.len() - at < quote.count as usize
                            && bytes[at..].iter().all(|b| *b == quote.quote))
                            || (quote.escape == Escape::Doubled && at + 1 == bytes.len()))
                    {
                        current.mode = Mode::Quote(quote);
                        styles.push(start, at, STYLE_STRING);
                        break 'scan;
                    }
                    if quote.escaped {
                        quote.escaped = false;
                        at += text[at..].chars().next()?.len_utf8();
                    } else if bytes[at] == b'\\' && quote.escape == Escape::Backslash {
                        quote.escaped = true;
                        at += 1;
                    } else if bytes[at..].starts_with(&[quote.quote; 3][..quote.count as usize]) {
                        if quote.escape == Escape::Doubled
                            && bytes.get(at + 1) == Some(&quote.quote)
                        {
                            at += 2;
                            continue;
                        }
                        at += quote.count as usize;
                        closed = true;
                        break;
                    } else if bytes[at] == b'\n' && !quote.multiline {
                        closed = true;
                        break;
                    } else {
                        at += text[at..].chars().next()?.len_utf8();
                    }
                }
                current.mode = if closed {
                    Mode::Normal
                } else {
                    Mode::Quote(quote)
                };
                STYLE_STRING
            }
            Mode::Raw(hashes) => {
                let mut closed = false;
                while at < bytes.len() {
                    if poll.cancelled(at) {
                        return None;
                    }
                    if !final_chunk
                        && bytes[at] == b'"'
                        && bytes.len() - at < 1 + hashes as usize
                        && bytes[at + 1..].iter().all(|b| *b == b'#')
                    {
                        styles.push(start, at, STYLE_STRING);
                        break 'scan;
                    }
                    if bytes[at] == b'"'
                        && bytes[at + 1..].starts_with(&[b'#'; 16][..hashes as usize])
                    {
                        at += 1 + hashes as usize;
                        closed = true;
                        break;
                    }
                    at += text[at..].chars().next()?.len_utf8();
                }
                if closed {
                    current.mode = Mode::Normal;
                }
                STYLE_STRING
            }
            Mode::Normal => {
                let c_style = matches!(
                    language,
                    Language::Rust
                        | Language::C
                        | Language::Cpp
                        | Language::JavaScript
                        | Language::TypeScript
                        | Language::Java
                        | Language::CSharp
                        | Language::Go
                        | Language::Swift
                        | Language::Kotlin
                );
                let hash_style = matches!(
                    language,
                    Language::Python | Language::Shell | Language::Toml | Language::Ruby
                );
                let batch_comment = language == Language::Batch
                    && current.command_start != BatchStart::Argument
                    && (bytes[at..].starts_with(b"::")
                        || (bytes
                            .get(at..at + 3)
                            .is_some_and(|word| word.eq_ignore_ascii_case(b"rem"))
                            && bytes.get(at + 3).is_none_or(u8::is_ascii_whitespace)));
                if !final_chunk
                    && ((c_style && &bytes[at..] == b"/")
                        || (language == Language::Sql && &bytes[at..] == b"-")
                        || (language == Language::Batch
                            && current.command_start != BatchStart::Argument
                            && &bytes[at..] == b":"))
                {
                    break;
                }
                if !final_chunk && language == Language::Cpp && cpp::incomplete_start(&bytes[at..])
                {
                    break;
                }
                if !final_chunk && language == Language::Rust && incomplete_rust_raw(&bytes[at..]) {
                    break;
                }
                if !final_chunk
                    && matches!(
                        language,
                        Language::Python
                            | Language::Toml
                            | Language::Java
                            | Language::Swift
                            | Language::CSharp
                            | Language::Kotlin
                    )
                    && matches!(bytes[at], b'\'' | b'"')
                    && bytes.len() - at < 3
                    && bytes[at..].iter().all(|b| *b == bytes[at])
                {
                    break;
                }
                if !final_chunk
                    && ((language == Language::CSharp && matches!(&bytes[at..], b"@" | b"@$"))
                        || (language == Language::Rust
                            && bytes[at] == b'\''
                            && text[at..].chars().take(3).count() <= 2
                            && !text[at..].contains('\n')))
                {
                    break;
                }
                if !final_chunk
                    && batch_comment
                    && bytes.len() - at == 3
                    && bytes[at..].eq_ignore_ascii_case(b"rem")
                {
                    break;
                }
                if language == Language::Cpp
                    && let Some((raw, length)) = cpp::start(&bytes[at..])
                {
                    current.mode = Mode::CppRaw(raw);
                    current.command_start = BatchStart::Argument;
                    at += length;
                    STYLE_STRING
                } else if batch_comment
                    || (c_style && bytes[at..].starts_with(b"//"))
                    || (hash_style && bytes[at] == b'#')
                    || (language == Language::Sql && bytes[at..].starts_with(b"--"))
                {
                    current.mode = Mode::LineComment;
                    continue;
                } else if (c_style || language == Language::Sql) && bytes[at..].starts_with(b"/*") {
                    at += 2;
                    current.mode = Mode::Block(1);
                    STYLE_COMMENT
                } else if language == Language::Rust
                    && bytes[at] == b'r'
                    && let Some(hashes) = raw_start(&bytes[at..])
                {
                    at += 2 + hashes as usize;
                    current.mode = Mode::Raw(hashes);
                    STYLE_STRING
                } else if let Some(quote) = quote_start(language, text, at) {
                    at += quote.count as usize;
                    current.mode = Mode::Quote(quote);
                    current.command_start = BatchStart::Argument;
                    STYLE_STRING
                } else if language == Language::Rust && bytes[at] == b'\'' {
                    at += 1;
                    let name: usize = text[at..]
                        .chars()
                        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                        .map(char::len_utf8)
                        .sum();
                    at += name;
                    current.command_start = BatchStart::Argument;
                    if name == 0 { STYLE_PLAIN } else { STYLE_TYPE }
                } else if bytes[at].is_ascii_digit() {
                    if !final_chunk
                        && (&bytes[at..] == b"0" || bytes[at..].eq_ignore_ascii_case(b"0x"))
                    {
                        break;
                    }
                    // The fixed regular expression sees at most 64 bytes between
                    // cancellation checks; continuation is an explicit cursor.
                    let mut bound = (at + 64).min(text.len());
                    while !text.is_char_boundary(bound) {
                        bound -= 1;
                    }
                    let token = tokens().find(&text[at..bound])?;
                    let token = token.as_str();
                    current.mode =
                        Mode::Number(if token.starts_with("0x") || token.starts_with("0X") {
                            Number::Hex
                        } else if token.contains(['e', 'E']) {
                            Number::Exponent
                        } else if token.contains('.') {
                            Number::Fraction
                        } else {
                            Number::Integer
                        });
                    at += token.len();
                    current.command_start = BatchStart::Argument;
                    STYLE_NUMBER
                } else if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
                    let mut end = at + 1;
                    while bytes
                        .get(end)
                        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    {
                        if poll.cancelled(end) {
                            return None;
                        }
                        end += 1;
                    }
                    let token = &text[at..end];
                    if !final_chunk && end == bytes.len() {
                        if token.len() <= 32 {
                            break;
                        }
                        current.mode = Mode::Identifier;
                    }
                    at = end;
                    current.command_start = BatchStart::Argument;
                    if keyword(language, token) {
                        STYLE_KEYWORD
                    } else {
                        STYLE_PLAIN
                    }
                } else {
                    let ch = text[at..].chars().next()?;
                    current.command_start = current.command_start.next(ch);
                    at += ch.len_utf8();
                    STYLE_PLAIN
                }
            }
        };
        styles.push(start, at, role);
    }
    if poll.cancelled(at) {
        return None;
    }
    *state = current;
    Some(at)
}

fn incomplete_rust_raw(bytes: &[u8]) -> bool {
    bytes.first() == Some(&b'r') && bytes.len() <= 18 && bytes[1..].iter().all(|b| *b == b'#')
}

impl Number {
    fn advance<F: Fn() -> bool>(
        self,
        bytes: &[u8],
        mut at: usize,
        final_chunk: bool,
        poll: &mut Poll<F>,
    ) -> Option<(usize, Self, bool)> {
        let mut phase = self;
        while let Some(&byte) = bytes.get(at) {
            if poll.cancelled(at) {
                return None;
            }
            if byte == b'_'
                || (if phase == Self::Hex {
                    byte.is_ascii_hexdigit()
                } else {
                    byte.is_ascii_digit()
                })
            {
                at += 1;
                continue;
            }
            if (phase == Self::Integer && byte == b'.')
                || (matches!(phase, Self::Integer | Self::Fraction) && matches!(byte, b'e' | b'E'))
            {
                let mut next = at + 1;
                if byte != b'.' && bytes.get(next).is_some_and(|b| matches!(b, b'+' | b'-')) {
                    next += 1;
                }
                if next == bytes.len() && !final_chunk {
                    return Some((at, phase, false));
                }
                if bytes
                    .get(next)
                    .is_some_and(|b| b.is_ascii_digit() || *b == b'_')
                {
                    phase = if byte == b'.' {
                        Self::Fraction
                    } else {
                        Self::Exponent
                    };
                    at = next;
                    continue;
                }
            }
            return Some((at, phase, true));
        }
        Some((at, phase, false))
    }
}
