//! Bounded lexical syntax catalogue. No grammar or regex comes from file content.
use crate::styles::Styles;
use midnight_plugin_sdk::api::viewer::*;
use regex::{Regex, RegexBuilder};
use std::sync::OnceLock;

#[path = "syntax_stream.rs"]
mod common;
#[path = "syntax_cpp.rs"]
mod cpp;
#[path = "syntax_lua.rs"]
mod lua;
#[path = "syntax_xml.rs"]
mod xml;
#[path = "syntax_yaml.rs"]
mod yaml;

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
    Lua,
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
    Identifier,
    Number(common::Number),
    LineComment,
    CppRaw(cpp::Raw),
    Xml(xml::State),
    Lua(lua::State),
    Yaml(yaml::State),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    mode: Mode,
    command_start: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            mode: Mode::Normal,
            command_start: true,
        }
    }
}
impl State {
    pub(crate) fn pending_span(&self) -> Option<(u64, u64)> {
        match self.mode {
            Mode::Lua(state) if state.pending_style() => state.pending_span(),
            Mode::Xml(state) if state.pending_style() => state.pending_span(),
            Mode::Yaml(state) if state.pending_style() => state.pending_span(),
            _ => None,
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
        "lua" => Some(Lua),
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
        "lua" | "lua5.1" | "lua5.2" | "lua5.3" | "lua5.4" | "luajit" => Some(Lua),
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
    let mut current = *state;
    let mut styles = Styles::new(0..text.len(), false);
    scan(text, language, &mut current, &mut styles, cancelled)?;
    let (spans, _, clipped) = styles.finish();
    if clipped {
        return None;
    }
    *state = current;
    Some(spans)
}

pub(crate) fn scan(
    text: &str,
    language: Language,
    state: &mut State,
    styles: &mut Styles,
    cancelled: impl Fn() -> bool,
) -> Option<()> {
    (stream(text, language, state, styles, true, cancelled)? == text.len()).then_some(())
}

/// Incremental decoded input. Incomplete lexical suffixes belong to the caller.
pub(crate) fn stream(
    text: &str,
    language: Language,
    state: &mut State,
    styles: &mut Styles,
    final_chunk: bool,
    cancelled: impl Fn() -> bool,
) -> Option<usize> {
    if text.len() > 65536 || cancelled() {
        return None;
    }
    match language {
        Language::Lua => {
            let mut current = if let Mode::Lua(current) = state.mode {
                current
            } else {
                lua::State::default()
            };
            let consumed = if final_chunk {
                lua::scan(text, &mut current, styles, cancelled)?;
                text.len()
            } else {
                lua::stream(text, &mut current, styles, false, cancelled)?
            };
            state.mode = if current == lua::State::default() {
                Mode::Normal
            } else {
                Mode::Lua(current)
            };
            Some(consumed)
        }
        Language::Xml => {
            let mut current = if let Mode::Xml(current) = state.mode {
                current
            } else {
                xml::State::default()
            };
            let consumed = if final_chunk {
                xml::scan(text, &mut current, styles, cancelled)?;
                text.len()
            } else {
                xml::stream(text, &mut current, styles, false, cancelled)?
            };
            state.mode = if current == xml::State::default() {
                Mode::Normal
            } else {
                Mode::Xml(current)
            };
            Some(consumed)
        }
        Language::Yaml => {
            let mut current = if let Mode::Yaml(current) = state.mode {
                current
            } else {
                yaml::State::default()
            };
            let consumed = if final_chunk {
                yaml::scan(text, &mut current, styles, cancelled)?;
                text.len()
            } else {
                yaml::stream(text, &mut current, styles, false, cancelled)?
            };
            state.mode = if current == yaml::State::default() {
                Mode::Normal
            } else {
                Mode::Yaml(current)
            };
            Some(consumed)
        }
        _ => common::stream(text, language, state, styles, final_chunk, cancelled),
    }
}

#[cfg(test)]
#[path = "syntax_stream_tests.rs"]
mod stream_tests;

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
        Language::Sql => {
            "SELECT FROM WHERE AS JOIN LEFT RIGHT INNER OUTER ON GROUP BY ORDER HAVING LIMIT OFFSET INSERT INTO VALUES UPDATE SET DELETE CREATE TABLE DROP ALTER INDEX VIEW DISTINCT UNION ALL NULL TRUE FALSE AND OR NOT IS LIKE IN EXISTS CASE WHEN THEN ELSE END BEGIN COMMIT ROLLBACK"
        }
        Language::Yaml => "true false null True False Null TRUE FALSE NULL yes no on off",
        Language::Xml | Language::Lua => "",
    };
    words.split_ascii_whitespace().any(|word| {
        if matches!(language, Language::Sql | Language::Batch) {
            word.eq_ignore_ascii_case(token)
        } else {
            word == token
        }
    })
}
