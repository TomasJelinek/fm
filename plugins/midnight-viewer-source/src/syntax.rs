//! Bounded lexical syntax catalogue. No grammar or regex comes from file content.
use midnight_plugin_sdk::api::viewer::*;
use regex::{Regex, RegexBuilder};
use std::sync::OnceLock;

#[path = "syntax_xml.rs"]
mod xml;

/// Host cancellation callbacks cross the plugin ABI and take host locks, so the
/// lexers poll them at most once per this many input bytes.
const POLL_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Rust,
    C,
    Cpp,
    JavaScript,
    TypeScript,
    Python,
    Shell,
    Batch,
    Json,
    Toml,
    Java,
    CSharp,
    Go,
    Swift,
    Kotlin,
    Ruby,
    Sql,
    Yaml,
    Xml,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Normal,
    Block(u8),
    Quote(Quote),
    Raw(u8),
}

/// An open string literal closed by `count` (1 or 3) `quote` bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Quote {
    quote: u8,
    count: u8,
    escape: Escape,
    /// Single-line literals end at an unescaped newline.
    multiline: bool,
    escaped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Escape {
    /// `\` escapes the next character.
    Backslash,
    /// A doubled delimiter is one literal delimiter (SQL, YAML, C# verbatim).
    Doubled,
    /// Every character before the delimiter is literal.
    Verbatim,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    mode: Mode,
    xml: xml::State,
}

/// Styles of one lexed text. `truncated` reports more than `MAX_SPANS` styled
/// tokens: the extra spans are omitted, but the lexical state still advanced
/// over the whole text, so later checkpoints remain exact.
#[derive(Debug, Default)]
pub struct Highlight {
    pub spans: Vec<Span>,
    pub truncated: bool,
}

/// Calls a cancellation callback at most once per `POLL_BYTES` of input.
struct Poll<F> {
    cancelled: F,
    next: usize,
}

impl<F: Fn() -> bool> Poll<F> {
    fn new(cancelled: F) -> Self {
        Self { cancelled, next: 0 }
    }

    fn cancelled(&mut self, at: usize) -> bool {
        if at < self.next {
            return false;
        }
        self.next = at.saturating_add(POLL_BYTES);
        (self.cancelled)()
    }
}

/// Collects merged spans up to `MAX_SPANS`, recording any overflow.
#[derive(Default)]
struct Spans(Highlight);

impl Spans {
    fn push(&mut self, start: usize, end: usize, role: u16) {
        if start == end {
            return;
        }
        let spans = &mut self.0.spans;
        if let Some(last) = spans.last_mut()
            && last.end == start as u32
            && last.role == role
        {
            last.end = end as u32;
        } else if spans.len() < MAX_SPANS {
            spans.push(Span {
                start: start as u32,
                end: end as u32,
                role,
                flags: 0,
            });
        } else {
            self.0.truncated = true;
        }
    }
}

/// Whether a Batch `::`/`rem` would start a command: the text since the last
/// `&`, `|`, `(`, `)` or newline is whitespace with optional leading `@`s.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BatchStart {
    Command,
    At,
    AtSpace,
    Argument,
}

impl BatchStart {
    fn next(self, ch: char) -> Self {
        match (self, ch) {
            (_, '\n' | '&' | '|' | '(' | ')') => Self::Command,
            (Self::Command, ch) if ch.is_whitespace() => Self::Command,
            (Self::Command | Self::At, '@') => Self::At,
            (Self::At | Self::AtSpace, ch) if ch.is_whitespace() => Self::AtSpace,
            _ => Self::Argument,
        }
    }
}

pub fn detect(name: &str, prefix: &str) -> Option<Language> {
    use Language::*;
    let extension = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let found = match extension.as_str() {
        "rs" => Some(Rust),
        "c" | "h" => Some(C),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some(Cpp),
        "js" | "mjs" | "cjs" | "jsx" => Some(JavaScript),
        "ts" | "tsx" | "mts" | "cts" => Some(TypeScript),
        "py" | "pyw" => Some(Python),
        "sh" | "bash" | "zsh" => Some(Shell),
        "bat" | "cmd" => Some(Batch),
        "json" => Some(Json),
        "toml" => Some(Toml),
        "java" => Some(Java),
        "cs" => Some(CSharp),
        "go" => Some(Go),
        "swift" => Some(Swift),
        "kt" | "kts" => Some(Kotlin),
        "rb" => Some(Ruby),
        "sql" => Some(Sql),
        "yaml" | "yml" => Some(Yaml),
        "xml" | "xsd" | "xsl" | "xslt" | "xaml" | "csproj" | "fsproj" | "vbproj" | "vcxproj"
        | "props" | "targets" | "resx" => Some(Xml),
        _ => None,
    };
    if found.is_some() {
        return found;
    }
    let first = prefix.lines().next().unwrap_or("");
    if !first.starts_with("#!") || first.len() > 256 {
        return None;
    }
    let words: Vec<_> = first[2..].split_ascii_whitespace().collect();
    let command = if words.first().is_some_and(|word| word.ends_with("/env")) {
        words
            .iter()
            .skip(1)
            .find(|word| !word.starts_with('-'))
            .copied()
    } else {
        words.first().copied()
    }?;
    match command.rsplit('/').next()? {
        "python" | "python3" | "python2" => Some(Python),
        "sh" | "bash" | "zsh" | "dash" => Some(Shell),
        "node" | "nodejs" => Some(JavaScript),
        _ => None,
    }
}

fn tokens() -> &'static Regex {
    static TOKENS: OnceLock<Regex> = OnceLock::new();
    TOKENS.get_or_init(|| RegexBuilder::new(
        r"\A(?:[A-Za-z_][A-Za-z_0-9]*|(?:0[xX][0-9A-Fa-f_]+|[0-9][0-9_]*(?:\.[0-9_]+)?(?:[eE][+-]?[0-9_]+)?))"
    ).size_limit(128 * 1024).dfa_size_limit(128 * 1024).build().unwrap())
}

/// Lexes `text` from `state`. `None` means cancellation or an exhausted
/// nesting/length bound; `state` then keeps its previous value.
pub fn highlight(
    text: &str,
    language: Language,
    state: &mut State,
    cancelled: impl Fn() -> bool,
) -> Option<Highlight> {
    let mut poll = Poll::new(cancelled);
    if text.len() > 64 * 1024 || poll.cancelled(0) {
        return None;
    }
    if language == Language::Xml {
        return xml::highlight(text, &mut state.xml, &mut poll);
    }
    let mut current = *state;
    let mut spans = Spans::default();
    let bytes = text.as_bytes();
    let mut at = 0;
    // Each call starts at a line start, where a Batch command can begin.
    let mut batch = BatchStart::Command;
    while at < bytes.len() {
        if poll.cancelled(at) {
            return None;
        }
        let start = at;
        let role = match current.mode {
            Mode::Block(mut depth) => {
                while at < bytes.len() {
                    if poll.cancelled(at) {
                        return None;
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
                        at += text[at..].chars().next().unwrap().len_utf8();
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
                    let delimiter = &[quote.quote; 3][..quote.count as usize];
                    if quote.escaped {
                        quote.escaped = false;
                        at += text[at..].chars().next().unwrap().len_utf8();
                    } else if bytes[at] == b'\\' && quote.escape == Escape::Backslash {
                        quote.escaped = true;
                        at += 1;
                    } else if bytes[at..].starts_with(delimiter) {
                        if quote.escape == Escape::Doubled
                            && bytes.get(at + 1) == Some(&quote.quote)
                        {
                            at += 2;
                            continue;
                        }
                        at += delimiter.len();
                        closed = true;
                        break;
                    } else if bytes[at] == b'\n' && !quote.multiline {
                        // An unterminated single-line literal ends with its line.
                        closed = true;
                        break;
                    } else {
                        at += text[at..].chars().next().unwrap().len_utf8();
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
                    if bytes[at] == b'"'
                        && bytes[at + 1..].starts_with(&[b'#'; 16][..hashes as usize])
                    {
                        at += 1 + hashes as usize;
                        closed = true;
                        break;
                    }
                    at += text[at..].chars().next().unwrap().len_utf8();
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
                    Language::Python
                        | Language::Shell
                        | Language::Toml
                        | Language::Ruby
                        | Language::Yaml
                );
                let batch_comment = language == Language::Batch
                    && batch != BatchStart::Argument
                    && (bytes[at..].starts_with(b"::")
                        || (bytes
                            .get(at..at + 3)
                            .is_some_and(|word| word.eq_ignore_ascii_case(b"rem"))
                            && bytes.get(at + 3).is_none_or(u8::is_ascii_whitespace)));
                if batch_comment
                    || (c_style && bytes[at..].starts_with(b"//"))
                    || (hash_style && bytes[at] == b'#')
                    || (language == Language::Sql && bytes[at..].starts_with(b"--"))
                {
                    at += text[at..].find('\n').unwrap_or(bytes.len() - at);
                    STYLE_COMMENT
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
                    STYLE_STRING
                } else if language == Language::Rust && bytes[at] == b'\'' {
                    // A lifetime or loop label, never a character literal.
                    at += 1;
                    let name: usize = text[at..]
                        .chars()
                        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                        .map(char::len_utf8)
                        .sum();
                    at += name;
                    if name == 0 { STYLE_PLAIN } else { STYLE_TYPE }
                } else if let Some(token) = tokens().find(&text[at..]) {
                    let token = token.as_str();
                    at += token.len();
                    if token.as_bytes()[0].is_ascii_digit() {
                        STYLE_NUMBER
                    } else if keyword(language, token) {
                        STYLE_KEYWORD
                    } else {
                        STYLE_PLAIN
                    }
                } else {
                    at += text[at..].chars().next().unwrap().len_utf8();
                    STYLE_PLAIN
                }
            }
        };
        if role != STYLE_PLAIN {
            spans.push(start, at, role);
        }
        if language == Language::Batch {
            batch = text[start..at].chars().fold(batch, BatchStart::next);
        }
    }
    *state = current;
    Some(spans.0)
}

/// The string literal that a delimiter at `at` opens in `language`, if any.
fn quote_start(language: Language, text: &str, at: usize) -> Option<Quote> {
    use Escape::*;
    use Language::*;
    let bytes = text.as_bytes();
    let quote = bytes[at];
    if !matches!(quote, b'"' | b'\'' | b'`') {
        return None;
    }
    let triple = bytes[at..].starts_with(&[quote; 3]);
    let (count, escape, multiline) = match (language, quote) {
        (Python, b'"' | b'\'') | (Toml | Java | Swift, b'"') if triple => (3, Backslash, true),
        (Toml, b'\'') | (CSharp | Kotlin, b'"') if triple => (3, Verbatim, true),
        (Rust | Ruby | Shell, b'"') | (Ruby, b'\'') | (JavaScript | TypeScript, b'`') => {
            (1, Backslash, true)
        }
        (Rust, b'\'') if char_literal(text, at) => (1, Backslash, false),
        (Shell, b'\'') | (Go, b'`') => (1, Verbatim, true),
        (Toml, b'\'') => (1, Verbatim, false),
        (CSharp, b'"') if bytes[..at].ends_with(b"@") || bytes[..at].ends_with(b"@$") => {
            (1, Doubled, true)
        }
        (Sql, b'"' | b'\'') => (1, Doubled, true),
        (Yaml, b'"') if yaml_scalar_start(bytes, at) => (1, Backslash, true),
        (Yaml, b'\'') if yaml_scalar_start(bytes, at) => (1, Doubled, true),
        (Batch, b'"') => (1, Verbatim, false),
        (Json | Toml, b'"')
        | (
            C | Cpp | JavaScript | TypeScript | Python | Java | CSharp | Go | Swift | Kotlin,
            b'"' | b'\'',
        ) => (1, Backslash, false),
        _ => return None,
    };
    Some(Quote {
        quote,
        count,
        escape,
        multiline,
        escaped: false,
    })
}

/// A Rust character literal (`'x'` or an escape), as opposed to a lifetime.
fn char_literal(text: &str, at: usize) -> bool {
    let mut chars = text[at + 1..].chars();
    match chars.next() {
        Some('\\') => true,
        Some('\'' | '\n') | None => false,
        Some(_) => chars.next() == Some('\''),
    }
}

/// YAML quotes delimit only whole scalars, not apostrophes inside plain text.
fn yaml_scalar_start(bytes: &[u8], at: usize) -> bool {
    let before = bytes[..at].trim_ascii_end();
    let gap = &bytes[before.len()..at];
    match before.last() {
        None => true,
        _ if gap.contains(&b'\n') => true,
        Some(b'[' | b'{' | b',') => true,
        Some(b':' | b'-' | b'?') => !gap.is_empty(),
        Some(_) => false,
    }
}

fn raw_start(bytes: &[u8]) -> Option<u8> {
    let mut hashes = 0;
    while bytes.get(1 + hashes) == Some(&b'#') {
        hashes += 1;
        if hashes > 16 {
            return None;
        }
    }
    (bytes.get(1 + hashes) == Some(&b'"')).then_some(hashes as u8)
}

fn keyword_list(language: Language) -> &'static str {
    match language {
        Language::Rust => {
            "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while"
        }
        Language::C => {
            "auto break case char const continue default do double else enum extern float for goto if inline int long register restrict return short signed sizeof static struct switch typedef union unsigned void volatile while _Bool"
        }
        Language::Cpp => {
            "alignas alignof auto bool break case catch char class const constexpr continue decltype default delete do double else enum explicit export extern false float for friend if inline int long namespace new noexcept nullptr operator private protected public register reinterpret_cast return short signed sizeof static static_cast struct switch template this throw true try typedef typename union unsigned using virtual void volatile while"
        }
        Language::JavaScript => {
            "async await break case catch class const continue debugger default delete do else export extends false finally for function if import in instanceof let new null of return static super switch this throw true try typeof var void while with yield"
        }
        Language::TypeScript => {
            "abstract any as async await boolean break case catch class const constructor continue declare default delete do else enum export extends false finally for from function if implements import in infer instanceof interface is keyof let module namespace never new null number of private protected public readonly return static string super switch this throw true try type typeof undefined unknown var void while yield"
        }
        Language::Python => {
            "False None True and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield"
        }
        Language::Shell => {
            "case do done elif else esac fi for function if in select then time until while"
        }
        Language::Batch => {
            "assoc break call cd chdir cls color copy date del dir do echo else endlocal equ errorlevel exist exit for ftype geq goto gtr if in leq lss md mkdir move neq not path pause popd prompt pushd rd rem ren rename rmdir set setlocal shift start time title type ver verify vol"
        }
        Language::Json => "false null true",
        Language::Toml => "false true inf nan",
        Language::Java => {
            "abstract assert boolean break byte case catch char class const continue default do double else enum extends final finally float for if implements import instanceof int interface long native new package private protected public record return short static strictfp super switch synchronized this throw throws transient try var void volatile while true false null"
        }
        Language::CSharp => {
            "abstract as async await base bool break byte case catch char checked class const continue decimal default delegate do double else enum event explicit extern false finally fixed float for foreach goto if implicit in int interface internal is lock long namespace new null object operator out override params private protected public readonly ref return sbyte sealed short sizeof stackalloc static string struct switch this throw true try typeof uint ulong unchecked unsafe ushort using var virtual void volatile while yield"
        }
        Language::Go => {
            "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var true false nil"
        }
        Language::Swift => {
            "associatedtype break case catch class continue default defer deinit do else enum extension fallthrough false fileprivate for func guard if import in init inout internal is let nil open operator private protocol public repeat rethrows return self static struct subscript super switch throw throws true try typealias var where while"
        }
        Language::Kotlin => {
            "as break class continue do else false for fun if in interface is null object package return super this throw true try typealias typeof val var when while by catch constructor delegate dynamic field file finally get import init param property receiver set setparam where actual abstract annotation companion const crossinline data enum expect external final infix inline inner internal lateinit noinline open operator out override private protected public reified sealed suspend tailrec vararg"
        }
        Language::Ruby => {
            "BEGIN END alias and begin break case class def defined do else elsif end ensure false for if in module next nil not or redo rescue retry return self super then true undef unless until when while yield"
        }
        // Case-insensitive languages list lowercase words.
        Language::Sql => {
            "select from where as join left right inner outer on group by order having limit offset insert into values update set delete create table drop alter index view distinct union all null true false and or not is like in exists case when then else end begin commit rollback"
        }
        Language::Yaml => "true false null True False Null TRUE FALSE NULL yes no on off",
        Language::Xml => "",
    }
}

fn keyword(language: Language, token: &str) -> bool {
    const LANGUAGES: usize = Language::Xml as usize + 1;
    static TABLES: [OnceLock<Box<[&'static str]>>; LANGUAGES] =
        [const { OnceLock::new() }; LANGUAGES];
    let words = TABLES[language as usize].get_or_init(|| {
        let mut words: Vec<_> = keyword_list(language).split_ascii_whitespace().collect();
        words.sort_unstable();
        words.into_boxed_slice()
    });
    if !matches!(language, Language::Sql | Language::Batch) {
        return words.binary_search(&token).is_ok();
    }
    // Tokens are ASCII; anything longer than the buffer is longer than every keyword.
    let mut lower = [0; 16];
    let Some(lower) = lower.get_mut(..token.len()) else {
        return false;
    };
    lower.copy_from_slice(token.as_bytes());
    lower.make_ascii_lowercase();
    std::str::from_utf8(lower).is_ok_and(|lower| words.binary_search(&lower).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_insensitive_keyword_tables_fit_the_lowercase_buffer() {
        for language in [Language::Sql, Language::Batch] {
            for word in keyword_list(language).split_ascii_whitespace() {
                assert!(
                    word.len() <= 16 && word == word.to_ascii_lowercase(),
                    "{word}"
                );
                assert!(keyword(language, &word.to_ascii_uppercase()));
            }
        }
        assert!(!keyword(Language::Sql, "selection"));
        assert!(keyword(Language::Rust, "Self") && !keyword(Language::Rust, "SELF"));
    }
}
