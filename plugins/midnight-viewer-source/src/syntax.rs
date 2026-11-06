//! Bounded lexical syntax catalogue. No grammar or regex comes from file content.
use midnight_plugin_sdk::api::viewer::*;
use regex::{Regex, RegexBuilder};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Rust,
    C,
    Cpp,
    JavaScript,
    TypeScript,
    Python,
    Shell,
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
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Normal,
    Block(u8),
    Quote {
        quote: u8,
        count: u8,
        escaped: bool,
    },
    Raw(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    mode: Mode,
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

pub fn highlight(
    text: &str,
    language: Language,
    state: &mut State,
    cancelled: impl Fn() -> bool,
) -> Option<Vec<Span>> {
    if text.len() > 64 * 1024 || cancelled() {
        return None;
    }
    let mut current = *state;
    let mut spans = Vec::<Span>::new();
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if cancelled() || spans.len() >= MAX_SPANS {
            return None;
        }
        let start = at;
        let role = match current.mode {
            Mode::Block(mut depth) => {
                while at < bytes.len() {
                    if at % 128 == 0 && cancelled() {
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
            Mode::Quote {
                quote,
                count,
                mut escaped,
            } => {
                let mut closed = false;
                while at < bytes.len() {
                    if at % 128 == 0 && cancelled() {
                        return None;
                    }
                    if escaped {
                        escaped = false;
                        at += text[at..].chars().next().unwrap().len_utf8();
                    } else if bytes[at] == b'\\' {
                        escaped = true;
                        at += 1;
                    } else if bytes[at..].starts_with(&[quote; 3][..count as usize]) {
                        at += count as usize;
                        closed = true;
                        break;
                    } else {
                        at += text[at..].chars().next().unwrap().len_utf8();
                    }
                }
                current.mode = if closed {
                    Mode::Normal
                } else {
                    Mode::Quote {
                        quote,
                        count,
                        escaped,
                    }
                };
                STYLE_STRING
            }
            Mode::Raw(hashes) => {
                let mut closed = false;
                while at < bytes.len() {
                    if at % 128 == 0 && cancelled() {
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
                if (c_style && bytes[at..].starts_with(b"//"))
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
                    && raw_start(&bytes[at..]).is_some()
                {
                    let hashes = raw_start(&bytes[at..]).unwrap();
                    at += 2 + hashes as usize;
                    current.mode = Mode::Raw(hashes);
                    STYLE_STRING
                } else if matches!(bytes[at], b'"' | b'\'')
                    || (matches!(language, Language::JavaScript | Language::TypeScript)
                        && bytes[at] == b'`')
                {
                    let quote = bytes[at];
                    let count = if matches!(language, Language::Python | Language::Toml)
                        && bytes[at..].starts_with(&[quote; 3])
                    {
                        3
                    } else {
                        1
                    };
                    at += count as usize;
                    current.mode = Mode::Quote {
                        quote,
                        count,
                        escaped: false,
                    };
                    STYLE_STRING
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
            if let Some(last) = spans
                .last_mut()
                .filter(|s| s.end as usize == start && s.role == role)
            {
                last.end = at as u32;
            } else {
                spans.push(Span {
                    start: start as u32,
                    end: at as u32,
                    role,
                    flags: 0,
                });
            }
        }
    }
    *state = current;
    Some(spans)
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

fn keyword(language: Language, token: &str) -> bool {
    let words = match language {
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
        Language::Sql => {
            "SELECT FROM WHERE AS JOIN LEFT RIGHT INNER OUTER ON GROUP BY ORDER HAVING LIMIT OFFSET INSERT INTO VALUES UPDATE SET DELETE CREATE TABLE DROP ALTER INDEX VIEW DISTINCT UNION ALL NULL TRUE FALSE AND OR NOT IS LIKE IN EXISTS CASE WHEN THEN ELSE END BEGIN COMMIT ROLLBACK"
        }
        Language::Yaml => "true false null True False Null TRUE FALSE NULL yes no on off",
    };
    words.split_ascii_whitespace().any(|word| {
        if language == Language::Sql {
            word.eq_ignore_ascii_case(token)
        } else {
            word == token
        }
    })
}
