//! Incremental common profiles. Short ambiguous suffixes belong to the caller.
use super::words::NAME_BYTES;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Number {
    Integer,
    Fraction,
    Exponent,
    Hex,
    /// The fraction of a hexadecimal floating literal such as `0x1.8p3`.
    HexFraction,
    /// Letters and digits after a value: a type suffix (`10u`, `1.5f`, `7i64`, `10n`)
    /// or the digits after a base prefix (`0b1010`, `0o17`).
    Suffix,
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
            Mode::Identifier(role) => {
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
                role
            }
            Mode::Number(number) => {
                let (end, number, done) =
                    number.advance(language, bytes, at, final_chunk, &mut poll)?;
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
            Mode::Xml(_)
            | Mode::Lua(_)
            | Mode::Yaml(_)
            | Mode::Usda(_)
            | Mode::Css(_)
            | Mode::Make(_)
            | Mode::Csv(_) => {
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
            // The commonest bytes open nothing in any common profile: runs of spaces
            // and tabs, and punctuation without a literal, comment or name context.
            Mode::Normal if matches!(bytes[at], b' ' | b'\t') => {
                while matches!(bytes.get(at), Some(b' ' | b'\t')) {
                    if poll.cancelled(at) {
                        return None;
                    }
                    at += 1;
                }
                current.command_start = current.command_start.next(' ');
                if matches!(
                    current.expect,
                    Expect::Member | Expect::Dots | Expect::Decorator
                ) {
                    current.expect = Expect::Nothing;
                }
                STYLE_PLAIN
            }
            Mode::Normal if inert(bytes[at]) => {
                current.command_start = current.command_start.next(char::from(bytes[at]));
                current.expect = Expect::Nothing;
                at += 1;
                STYLE_PLAIN
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
                // Every raw string prefix starts with one of these letters.
                let raw_prefix = matches!(bytes[at], b'R' | b'u' | b'U' | b'L');
                if !final_chunk
                    && language == Language::Cpp
                    && raw_prefix
                    && cpp::incomplete_start(&bytes[at..])
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
                    && raw_prefix
                    && let Some((raw, length)) = cpp::start(&bytes[at..])
                {
                    current.mode = Mode::CppRaw(raw);
                    current.command_start = BatchStart::Argument;
                    current.expect = Expect::Nothing;
                    at += length;
                    STYLE_STRING
                } else if (bytes[at].is_ascii_alphabetic() || bytes[at] == b'_')
                    && !batch_comment
                    && !(language == Language::Rust
                        && bytes[at] == b'r'
                        && raw_start(&bytes[at..]).is_some())
                {
                    // A name; only raw strings and Batch `rem` start with a letter too.
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
                    if end - at > NAME_BYTES {
                        // Too long to classify; a continuation stays plain.
                        if !final_chunk && end == bytes.len() {
                            current.mode = Mode::Identifier(STYLE_PLAIN);
                        }
                        at = end;
                        current.command_start = BatchStart::Argument;
                        current.expect = Expect::Nothing;
                        STYLE_PLAIN
                    } else {
                        let Some(name) =
                            classify(language, text, at, end, current.expect, final_chunk)
                        else {
                            break;
                        };
                        at = name.end;
                        current.command_start = BatchStart::Argument;
                        current.expect = name.expect;
                        name.role
                    }
                } else if language == Language::Shell && bytes[at] == b'$' {
                    // Before `#` comments: `$#` is an expansion.
                    let found = expansion(bytes, at, final_chunk, &mut poll)?;
                    if found == Expansion::Wait {
                        break;
                    }
                    current.command_start = BatchStart::Argument;
                    current.expect = Expect::Nothing;
                    match found {
                        Expansion::End(end) => {
                            at = end;
                            STYLE_TYPE
                        }
                        Expansion::Continues(end) => {
                            at = end;
                            current.mode = Mode::Identifier(STYLE_TYPE);
                            STYLE_TYPE
                        }
                        Expansion::Wait | Expansion::Plain => {
                            at += 1;
                            STYLE_PLAIN
                        }
                    }
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
                    current.expect = Expect::Nothing;
                    STYLE_STRING
                } else if let Some(quote) = quote_start(language, text, at) {
                    at += quote.count as usize;
                    current.mode = Mode::Quote(quote);
                    current.command_start = BatchStart::Argument;
                    current.expect = Expect::Nothing;
                    STYLE_STRING
                } else if current.expect == Expect::Header && bytes[at] == b'<' {
                    // `#include <path>`: the header name is a single-line string.
                    at += 1;
                    current.mode = Mode::Quote(Quote {
                        quote: b'>',
                        count: 1,
                        escape: Escape::Verbatim,
                        multiline: false,
                        escaped: false,
                    });
                    current.command_start = BatchStart::Argument;
                    current.expect = Expect::Nothing;
                    STYLE_STRING
                } else if bytes[at] == b'#'
                    && matches!(
                        language,
                        Language::C | Language::Cpp | Language::CSharp | Language::Swift
                    )
                {
                    // `#`, at most 16 spaces or tabs, then a directive name.
                    let mut name = at + 1;
                    while name - at <= 16 && matches!(bytes.get(name), Some(b' ' | b'\t')) {
                        name += 1;
                    }
                    let mut end = name;
                    while end - name <= 16
                        && bytes
                            .get(end)
                            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    {
                        end += 1;
                    }
                    if !final_chunk && end == bytes.len() {
                        break;
                    }
                    current.command_start = BatchStart::Argument;
                    let directive = &text[name..end];
                    if words::directive(language, directive) {
                        current.expect = if words::header_directive(directive) {
                            Expect::Header
                        } else {
                            Expect::Nothing
                        };
                        at = end;
                        STYLE_KEYWORD
                    } else {
                        current.expect = Expect::Nothing;
                        at += 1;
                        STYLE_PLAIN
                    }
                } else if language == Language::Python
                    && bytes[at] == b'@'
                    && current.command_start != BatchStart::Argument
                {
                    // A decorator: `@` first on its line, then a dotted name.
                    at += 1;
                    current.command_start = BatchStart::Argument;
                    current.expect = Expect::Decorator;
                    STYLE_FUNCTION
                } else if language == Language::Rust && bytes[at] == b'\'' {
                    at += 1;
                    let name: usize = text[at..]
                        .chars()
                        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                        .map(char::len_utf8)
                        .sum();
                    at += name;
                    current.command_start = BatchStart::Argument;
                    current.expect = Expect::Nothing;
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
                    current.expect = Expect::Nothing;
                    STYLE_NUMBER
                } else {
                    let ch = text[at..].chars().next()?;
                    if ch == '.'
                        && current.expect == Expect::Type
                        && !final_chunk
                        && bytes.len() - at < 3
                    {
                        // `...` after a declaring keyword is variadic; one `.` is a member.
                        break;
                    }
                    current.command_start = current.command_start.next(ch);
                    at += ch.len_utf8();
                    let mut role = STYLE_PLAIN;
                    current.expect = match (ch, current.expect) {
                        (
                            ' ' | '\t' | '\r' | '\x0c',
                            Expect::Member | Expect::Dots | Expect::Decorator,
                        )
                        | (
                            '\n',
                            Expect::Member | Expect::Dots | Expect::Decorator | Expect::Header,
                        ) => Expect::Nothing,
                        (' ' | '\t' | '\r' | '\x0c' | '\n', expect) => expect,
                        (':', expect)
                            if matches!(language, Language::Cpp | Language::Rust)
                                && bytes.get(at) == Some(&b':') =>
                        {
                            at += 1;
                            match expect {
                                Expect::StdScope => Expect::Std,
                                Expect::Type | Expect::TypePath => Expect::Type,
                                _ => Expect::Nothing,
                            }
                        }
                        ('.', Expect::Type) if bytes[at..].starts_with(b"..") => {
                            at += 2;
                            Expect::Type
                        }
                        ('.', Expect::TypePath) => Expect::Type,
                        ('.', Expect::Decorator) => {
                            role = STYLE_FUNCTION;
                            Expect::Decorator
                        }
                        ('.', Expect::Member | Expect::Dots) => Expect::Dots,
                        ('.', _) => Expect::Member,
                        _ => Expect::Nothing,
                    };
                    role
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

/// A classified name and the end of its styled run (a Rust macro's `!` included).
struct Name {
    end: usize,
    role: u16,
    expect: Expect,
}

/// Classify the name `text[at..end]`, at most `NAME_BYTES` long. `None`: its role
/// depends on bytes after the end of a non-final chunk.
fn classify(
    language: Language,
    text: &str,
    at: usize,
    end: usize,
    expect: Expect,
    final_chunk: bool,
) -> Option<Name> {
    let bytes = text.as_bytes();
    let token = &text[at..end];
    // Up to three following bytes decide: `(`, `::`, `::<`, `!`, a quote or `.`.
    let missing = |ahead: usize| !final_chunk && end + ahead >= bytes.len();
    if missing(0) {
        return None;
    }
    let next = bytes.get(end).copied();
    let member = expect == Expect::Member;
    let name = |role, expect| Some(Name { end, role, expect });
    // `L"wide"`, `u8'c'`, `b"bytes"`, `f"text"`: the prefix belongs to the literal.
    if !member && matches!(next, Some(b'"' | b'\'')) && string_prefix(language, token) {
        if language == Language::Rust
            && next == Some(b'\'')
            && !final_chunk
            && text[end..].chars().take(3).count() <= 2
            && !text[end..].contains('\n')
        {
            return None;
        }
        if quote_start(language, text, end).is_some() {
            return name(STYLE_STRING, Expect::Nothing);
        }
    }
    if (!member || words::member_keyword(language, token))
        && let Some(word) = words::word(language, token)
        && (!member || word.role == STYLE_KEYWORD)
    {
        return name(word.role, word.expect);
    }
    let colon = matches!(language, Language::Cpp | Language::Rust) && next == Some(b':');
    if colon && missing(1) {
        return None;
    }
    let qualified = colon && bytes.get(end + 1) == Some(&b':');
    if qualified && language == Language::Rust && missing(2) {
        return None;
    }
    let turbofish = qualified && language == Language::Rust && bytes.get(end + 2) == Some(&b'<');
    let bang = language == Language::Rust && next == Some(b'!');
    if bang && missing(1) {
        return None;
    }
    let c_family = matches!(language, Language::C | Language::Cpp);
    // What the name makes of the next one: qualifiers continue a context.
    let after =
        if qualified && ((language == Language::Cpp && token == "std") || expect == Expect::Std) {
            Expect::StdScope
        } else if expect == Expect::Type && (qualified || next == Some(b'.')) {
            Expect::TypePath
        } else if expect == Expect::Decorator {
            Expect::Decorator
        } else {
            Expect::Nothing
        };
    // POSIX reserves the `_t` suffix for types: `size_t`, `uint64_t`, `pthread_t`.
    let role = if (c_family && !member && token.len() > 2 && token.ends_with("_t"))
        || (expect == Expect::Std && words::std_type(token))
    {
        STYLE_TYPE
    } else if expect == Expect::Type {
        if c_family && macro_like(token) {
            // An export or attribute macro: `class DLL_EXPORT Name`.
            return name(STYLE_PLAIN, Expect::Type);
        }
        STYLE_TYPE
    } else if expect == Expect::Decorator {
        STYLE_FUNCTION
    } else if bang && bytes.get(end + 1) != Some(&b'=') {
        // A Rust macro invocation, `!` included: `println!`, `vec!`.
        return Some(Name {
            end: end + 1,
            role: STYLE_FUNCTION,
            expect: Expect::Nothing,
        });
    } else if pascal_case(language, token) {
        STYLE_TYPE
    } else if expect == Expect::Function || turbofish || (next == Some(b'(') && calls(language)) {
        STYLE_FUNCTION
    } else if qualified {
        STYLE_TYPE
    } else {
        STYLE_PLAIN
    };
    name(role, after)
}

/// String prefixes that belong to a following literal.
fn string_prefix(language: Language, token: &str) -> bool {
    match language {
        Language::C | Language::Cpp => matches!(token, "L" | "u" | "U" | "u8"),
        Language::Rust => matches!(token, "b" | "c"),
        Language::Python => match *token.as_bytes() {
            [a] => matches!(a.to_ascii_lowercase(), b'b' | b'f' | b'r' | b't' | b'u'),
            [a, b] => {
                let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
                (a == b'r' && matches!(b, b'b' | b'f' | b't'))
                    || (b == b'r' && matches!(a, b'b' | b'f' | b't'))
            }
            _ => false,
        },
        _ => false,
    }
}

/// Languages whose type names, and only type names, are capitalized words; in
/// those with generics a lone capital letter is a type parameter.
fn pascal_case(language: Language, token: &str) -> bool {
    let single = match language {
        Language::Rust
        | Language::Java
        | Language::Kotlin
        | Language::Swift
        | Language::TypeScript => true,
        Language::Python | Language::JavaScript | Language::Ruby => false,
        _ => return false,
    };
    let bytes = token.as_bytes();
    bytes[0].is_ascii_uppercase()
        && ((single && bytes.len() == 1) || bytes[1..].iter().any(u8::is_ascii_lowercase))
}

/// `NAME_WITH_UNDERSCORES`: a macro by convention.
fn macro_like(token: &str) -> bool {
    token.contains('_')
        && token.bytes().any(|b| b.is_ascii_uppercase())
        && token
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// Punctuation that opens no literal, comment, name context or expansion in any
/// common profile; Batch command separators still update `BatchStart`.
fn inert(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')'
            | b'{'
            | b'}'
            | b'['
            | b']'
            | b';'
            | b','
            | b'='
            | b'+'
            | b'*'
            | b'&'
            | b'|'
            | b'>'
            | b'!'
            | b'?'
            | b'%'
            | b'^'
            | b'~'
    )
}

/// Languages where `name(` calls or declares a function.
fn calls(language: Language) -> bool {
    !matches!(language, Language::Batch | Language::Json | Language::Toml)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expansion {
    /// Not an expansion: `$(`, `$'`, a lone `$`.
    Plain,
    /// An expansion ending at this byte.
    End(usize),
    /// A name longer than `NAME_BYTES` that reaches the end of a non-final chunk.
    Continues(usize),
    /// The end depends on bytes after a non-final chunk.
    Wait,
}

/// A shell parameter expansion at `$`: `$name`, `$1`, `$?`, `${name}` or the start
/// of `${#name...}`. `None`: cancelled.
fn expansion<F: Fn() -> bool>(
    bytes: &[u8],
    at: usize,
    final_chunk: bool,
    poll: &mut Poll<F>,
) -> Option<Expansion> {
    let Some(&next) = bytes.get(at + 1) else {
        return Some(if final_chunk {
            Expansion::Plain
        } else {
            Expansion::Wait
        });
    };
    if next.is_ascii_digit() || matches!(next, b'@' | b'*' | b'#' | b'?' | b'$' | b'!' | b'-') {
        return Some(Expansion::End(at + 2));
    }
    let braced = next == b'{';
    let mut from = at + 1 + usize::from(braced);
    if braced && matches!(bytes.get(from), Some(b'#' | b'!')) {
        from += 1;
    }
    let mut end = from;
    while bytes
        .get(end)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        if poll.cancelled(end) {
            return None;
        }
        end += 1;
    }
    let long = end - from > NAME_BYTES;
    if !final_chunk && end == bytes.len() {
        return Some(if long {
            Expansion::Continues(end)
        } else {
            Expansion::Wait
        });
    }
    if end == from && !braced {
        return Some(Expansion::Plain);
    }
    if braced && !long && end > from && bytes.get(end) == Some(&b'}') {
        return Some(Expansion::End(end + 1));
    }
    Some(Expansion::End(end))
}

fn incomplete_rust_raw(bytes: &[u8]) -> bool {
    bytes.first() == Some(&b'r') && bytes.len() <= 18 && bytes[1..].iter().all(|b| *b == b'#')
}

impl Number {
    fn advance<F: Fn() -> bool>(
        self,
        language: Language,
        bytes: &[u8],
        mut at: usize,
        final_chunk: bool,
        poll: &mut Poll<F>,
    ) -> Option<(usize, Self, bool)> {
        // C23 and C++14 digit separators: `1'000'000`, `0b1010'0101`.
        let separators = matches!(language, Language::C | Language::Cpp);
        let hex_floats = matches!(
            language,
            Language::C | Language::Cpp | Language::Go | Language::Java | Language::Swift
        );
        let mut phase = self;
        while let Some(&byte) = bytes.get(at) {
            if poll.cancelled(at) {
                return None;
            }
            let digit = match phase {
                Self::Hex | Self::HexFraction => byte.is_ascii_hexdigit(),
                Self::Suffix => byte.is_ascii_alphanumeric(),
                _ => byte.is_ascii_digit(),
            };
            if byte == b'_' || digit {
                at += 1;
                continue;
            }
            if separators && byte == b'\'' {
                match bytes.get(at + 1) {
                    None if !final_chunk => return Some((at, phase, false)),
                    Some(next) if next.is_ascii_alphanumeric() || *next == b'_' => {
                        at += 1;
                        continue;
                    }
                    _ => return Some((at, phase, true)),
                }
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
            if hex_floats && phase == Self::Hex && byte == b'.' {
                match bytes.get(at + 1) {
                    None if !final_chunk => return Some((at, phase, false)),
                    Some(next) if next.is_ascii_hexdigit() || matches!(next, b'p' | b'P') => {
                        phase = Self::HexFraction;
                        at += 1;
                        continue;
                    }
                    _ => {}
                }
            }
            if hex_floats
                && matches!(phase, Self::Hex | Self::HexFraction)
                && matches!(byte, b'p' | b'P')
            {
                let mut next = at + 1;
                if bytes.get(next).is_some_and(|b| matches!(b, b'+' | b'-')) {
                    next += 1;
                }
                if next == bytes.len() && !final_chunk {
                    return Some((at, phase, false));
                }
                if bytes.get(next).is_some_and(u8::is_ascii_digit) {
                    phase = Self::Exponent;
                    at = next;
                    continue;
                }
            }
            if byte.is_ascii_alphabetic() {
                // Any letters and digits that follow belong to the literal's suffix.
                phase = Self::Suffix;
                at += 1;
                continue;
            }
            return Some((at, phase, true));
        }
        Some((at, phase, false))
    }
}
