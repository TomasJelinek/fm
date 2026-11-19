//! USDA lexical colouring; paths/assets are inert and no document is parsed or composed.
use midnight_plugin_sdk::api::viewer::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct State {
    mode: Mode,
    prim_type: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Normal,
    Identifier(u16),
    Number(Number),
    LineComment,
    Block {
        star: bool,
    },
    Quote {
        quote: u8,
        triple: bool,
        escaped: bool,
    },
    Asset {
        triple: bool,
    },
    Path,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Number {
    Integer,
    Fraction,
    Exponent,
}

fn identifier_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || !ch.is_ascii()
}

fn identifier_continue(ch: char) -> bool {
    identifier_start(ch) || ch.is_ascii_digit() || ch == ':'
}

fn keyword(word: &str) -> bool {
    matches!(
        word,
        "add"
            | "AnimationBlock"
            | "append"
            | "at"
            | "autoEase"
            | "bezier"
            | "class"
            | "config"
            | "connect"
            | "curve"
            | "custom"
            | "customData"
            | "default"
            | "defaultPrim"
            | "def"
            | "delete"
            | "dictionary"
            | "displayUnit"
            | "doc"
            | "edit"
            | "erase"
            | "false"
            | "fill"
            | "held"
            | "hermite"
            | "inherits"
            | "insert"
            | "kind"
            | "linear"
            | "loop"
            | "maxsize"
            | "minsize"
            | "nameChildren"
            | "None"
            | "none"
            | "offset"
            | "oscillate"
            | "over"
            | "payload"
            | "permission"
            | "post"
            | "prefixSubstitutions"
            | "pre"
            | "prepend"
            | "properties"
            | "references"
            | "rel"
            | "relocates"
            | "reorder"
            | "reset"
            | "resize"
            | "rootPrims"
            | "repeat"
            | "scale"
            | "sloped"
            | "specializes"
            | "spline"
            | "subLayers"
            | "suffixSubstitutions"
            | "symmetryArguments"
            | "symmetryFunction"
            | "timeSamples"
            | "to"
            | "true"
            | "uniform"
            | "variantSet"
            | "variantSets"
            | "variants"
            | "varying"
            | "write"
    )
}

fn value_type(word: &str) -> bool {
    matches!(
        word,
        "asset"
            | "bool"
            | "color3d"
            | "color3f"
            | "color3h"
            | "color4d"
            | "color4f"
            | "color4h"
            | "double"
            | "double2"
            | "double3"
            | "double4"
            | "float"
            | "float2"
            | "float3"
            | "float4"
            | "frame4d"
            | "group"
            | "half"
            | "half2"
            | "half3"
            | "half4"
            | "int"
            | "int2"
            | "int3"
            | "int4"
            | "int64"
            | "matrix2d"
            | "matrix3d"
            | "matrix4d"
            | "normal3d"
            | "normal3f"
            | "normal3h"
            | "opaque"
            | "point3d"
            | "point3f"
            | "point3h"
            | "quatd"
            | "quatf"
            | "quath"
            | "string"
            | "texCoord2d"
            | "texCoord2f"
            | "texCoord2h"
            | "texCoord3d"
            | "texCoord3f"
            | "texCoord3h"
            | "timecode"
            | "token"
            | "uchar"
            | "uint"
            | "uint64"
            | "vector3d"
            | "vector3f"
            | "vector3h"
    )
}

pub(super) fn scan(
    text: &str,
    state: &mut State,
    styles: &mut crate::styles::Styles,
    cancelled: impl Fn() -> bool,
) -> Option<()> {
    (stream(text, state, styles, true, cancelled)? == text.len()).then_some(())
}

/// The caller retains ambiguous suffixes (at most 32 UTF-8 bytes). Long tokens
/// continue in Copy state; cancellation never commits a partial checkpoint.
pub(super) fn stream(
    text: &str,
    state: &mut State,
    styles: &mut crate::styles::Styles,
    final_chunk: bool,
    cancelled: impl Fn() -> bool,
) -> Option<usize> {
    if text.len() > 65536 {
        return None;
    }
    let mut poll = super::Poll::new(cancelled);
    let mut current = *state;
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if poll.cancelled(at) {
            return None;
        }
        match current.mode {
            Mode::Normal => {
                let ch = text[at..].chars().next()?;
                let tail = &text[at..];
                if ch.is_whitespace() {
                    at += ch.len_utf8();
                    continue;
                }
                if bytes[at] == b'#' || tail.starts_with("//") {
                    let count = if bytes[at] == b'#' { 1 } else { 2 };
                    styles.push(at, at + count, STYLE_COMMENT);
                    at += count;
                    current.mode = Mode::LineComment;
                } else if tail.starts_with("/*") {
                    styles.push(at, at + 2, STYLE_COMMENT);
                    at += 2;
                    current.mode = Mode::Block { star: false };
                } else if tail == "/" && !final_chunk {
                    break;
                } else if matches!(bytes[at], b'\'' | b'"' | b'@') {
                    let delimiter = bytes[at];
                    let count = bytes[at..]
                        .iter()
                        .take(3)
                        .take_while(|&&b| b == delimiter)
                        .count();
                    if count < 3 && at + count == bytes.len() && !final_chunk {
                        break;
                    }
                    let triple = count == 3;
                    let count = if triple { 3 } else { 1 };
                    styles.push(at, at + count, STYLE_STRING);
                    at += count;
                    current.prim_type = false;
                    current.mode = if delimiter == b'@' {
                        Mode::Asset { triple }
                    } else {
                        Mode::Quote {
                            quote: delimiter,
                            triple,
                            escaped: false,
                        }
                    };
                } else if bytes[at] == b'<' {
                    styles.push(at, at + 1, STYLE_STRING);
                    at += 1;
                    current.prim_type = false;
                    current.mode = Mode::Path;
                } else if identifier_start(ch) {
                    let start = at;
                    let mut end = at;
                    for ch in tail.chars() {
                        if !identifier_continue(ch) {
                            break;
                        }
                        if poll.cancelled(end) {
                            return None;
                        }
                        end += ch.len_utf8();
                    }
                    if end == bytes.len() && !final_chunk && end - start <= 32 {
                        break;
                    }
                    let word = &text[start..end];
                    let role = if current.prim_type {
                        STYLE_TYPE
                    } else if keyword(word) {
                        STYLE_KEYWORD
                    } else if value_type(word) {
                        STYLE_TYPE
                    } else if matches!(word, "inf" | "nan") {
                        STYLE_NUMBER
                    } else {
                        STYLE_PLAIN
                    };
                    styles.push(start, end, role);
                    at = end;
                    current.prim_type = matches!(word, "def" | "over" | "class");
                    if at == bytes.len() && !final_chunk {
                        current.mode = Mode::Identifier(role);
                    }
                } else {
                    // Decimal/exponent numbers and -inf follow USDA syntax;
                    // unrelated names such as infSuffix remain ordinary names.
                    if bytes[at] == b'-' {
                        if "-inf".starts_with(tail) && !final_chunk {
                            break;
                        }
                        if tail.starts_with("-inf")
                            && (tail.len() == 4 || !identifier_continue(tail[4..].chars().next()?))
                        {
                            styles.push(at, at + 4, STYLE_NUMBER);
                            at += 4;
                            current.prim_type = false;
                            continue;
                        }
                    }
                    let sign = usize::from(bytes[at] == b'-');
                    let digit = bytes.get(at + sign).is_some_and(u8::is_ascii_digit);
                    let dot = bytes.get(at + sign) == Some(&b'.');
                    let fraction = dot && bytes.get(at + sign + 1).is_some_and(u8::is_ascii_digit);
                    if (tail == "-" || tail == "." || tail == "-.") && !final_chunk {
                        break;
                    }
                    if digit || fraction {
                        let count = sign + 1;
                        styles.push(at, at + count, STYLE_NUMBER);
                        at += count;
                        current.prim_type = false;
                        current.mode = Mode::Number(if fraction {
                            Number::Fraction
                        } else {
                            Number::Integer
                        });
                    } else {
                        if matches!(
                            bytes[at],
                            b'=' | b','
                                | b':'
                                | b';'
                                | b'$'
                                | b'.'
                                | b'['
                                | b']'
                                | b'('
                                | b')'
                                | b'{'
                                | b'}'
                                | b'&'
                                | b'-'
                                | b'+'
                        ) {
                            styles.push(at, at + 1, STYLE_OPERATOR);
                        }
                        current.prim_type = false;
                        at += ch.len_utf8();
                    }
                }
            }
            Mode::Identifier(role) => {
                let start = at;
                for ch in text[at..].chars() {
                    if !identifier_continue(ch) {
                        break;
                    }
                    if poll.cancelled(at) {
                        return None;
                    }
                    at += ch.len_utf8();
                }
                styles.push(start, at, role);
                if at < bytes.len() || final_chunk {
                    current.mode = Mode::Normal;
                }
            }
            Mode::Number(mut number) => {
                let start = at;
                let mut pending = false;
                while at < bytes.len() {
                    if poll.cancelled(at) {
                        return None;
                    }
                    if bytes[at].is_ascii_digit() {
                        at += 1;
                    } else if number == Number::Integer && bytes[at] == b'.' {
                        at += 1;
                        number = Number::Fraction;
                    } else if number != Number::Exponent && matches!(bytes[at], b'e' | b'E') {
                        let mut end = at + 1;
                        if bytes.get(end).is_some_and(|b| matches!(b, b'+' | b'-')) {
                            end += 1;
                        }
                        if end == bytes.len() && !final_chunk {
                            pending = true;
                            break;
                        }
                        if !bytes.get(end).is_some_and(u8::is_ascii_digit) {
                            break;
                        }
                        at = end;
                        number = Number::Exponent;
                    } else {
                        break;
                    }
                }
                styles.push(start, at, STYLE_NUMBER);
                current.mode = if at == bytes.len() && !final_chunk || pending {
                    Mode::Number(number)
                } else {
                    Mode::Normal
                };
                if pending {
                    break;
                }
            }
            Mode::LineComment => {
                let start = at;
                for ch in text[at..].chars() {
                    if matches!(ch, '\r' | '\n') {
                        break;
                    }
                    if poll.cancelled(at) {
                        return None;
                    }
                    at += ch.len_utf8();
                }
                styles.push(start, at, STYLE_COMMENT);
                if at < bytes.len() || final_chunk {
                    current.mode = Mode::Normal;
                }
            }
            Mode::Block { mut star } => {
                let start = at;
                let mut closed = false;
                while at < bytes.len() {
                    if poll.cancelled(at) {
                        return None;
                    }
                    let ch = text[at..].chars().next()?;
                    at += ch.len_utf8();
                    if star && ch == '/' {
                        closed = true;
                        break;
                    }
                    star = ch == '*';
                }
                styles.push(start, at, STYLE_COMMENT);
                current.mode = if closed {
                    Mode::Normal
                } else {
                    Mode::Block { star }
                };
            }
            Mode::Quote {
                quote,
                triple,
                mut escaped,
            } => {
                let start = at;
                let mut closed = false;
                let mut pending = false;
                while at < bytes.len() {
                    if poll.cancelled(at) {
                        return None;
                    }
                    if !triple && matches!(bytes[at], b'\r' | b'\n') {
                        closed = true;
                        break;
                    }
                    if escaped {
                        escaped = false;
                        at += text[at..].chars().next()?.len_utf8();
                    } else if bytes[at] == b'\\' {
                        escaped = true;
                        at += 1;
                    } else if bytes[at] == quote {
                        let count = bytes[at..]
                            .iter()
                            .take(3)
                            .take_while(|&&b| b == quote)
                            .count();
                        if triple && count < 3 && at + count == bytes.len() && !final_chunk {
                            pending = true;
                            break;
                        }
                        if !triple || count == 3 {
                            at += if triple { 3 } else { 1 };
                            closed = true;
                            break;
                        }
                        at += count;
                    } else {
                        at += text[at..].chars().next()?.len_utf8();
                    }
                }
                styles.push(start, at, STYLE_STRING);
                current.mode = if closed {
                    Mode::Normal
                } else {
                    Mode::Quote {
                        quote,
                        triple,
                        escaped,
                    }
                };
                if pending {
                    break;
                }
            }
            Mode::Asset { triple } => {
                let start = at;
                let mut closed = false;
                let mut pending = false;
                while at < bytes.len() {
                    if poll.cancelled(at) {
                        return None;
                    }
                    let tail = &text[at..];
                    if matches!(bytes[at], b'\r' | b'\n') {
                        closed = true;
                        break;
                    }
                    if triple && "\\@@@".starts_with(tail) && tail.len() < 4 && !final_chunk {
                        pending = true;
                        break;
                    }
                    if triple && tail.starts_with("\\@@@") {
                        at += 4;
                    } else if bytes[at] == b'@' {
                        let count = bytes[at..]
                            .iter()
                            .take(5)
                            .take_while(|&&b| b == b'@')
                            .count();
                        if triple && count < 5 && at + count == bytes.len() && !final_chunk {
                            pending = true;
                            break;
                        }
                        if !triple || count >= 3 {
                            at += if triple { count } else { 1 };
                            closed = true;
                            break;
                        }
                        at += count;
                    } else {
                        at += text[at..].chars().next()?.len_utf8();
                    }
                }
                styles.push(start, at, STYLE_STRING);
                current.mode = if closed {
                    Mode::Normal
                } else {
                    Mode::Asset { triple }
                };
                if pending {
                    break;
                }
            }
            Mode::Path => {
                let start = at;
                let mut closed = false;
                while at < bytes.len() {
                    if poll.cancelled(at) {
                        return None;
                    }
                    if matches!(bytes[at], b'\r' | b'\n' | b'<') {
                        closed = true;
                        break;
                    }
                    let ch = text[at..].chars().next()?;
                    at += ch.len_utf8();
                    if ch == '>' {
                        closed = true;
                        break;
                    }
                }
                styles.push(start, at, STYLE_STRING);
                current.mode = if closed { Mode::Normal } else { Mode::Path };
            }
        }
    }
    if final_chunk
        && matches!(
            current.mode,
            Mode::Identifier(_) | Mode::Number(_) | Mode::LineComment
        )
    {
        current.mode = Mode::Normal;
    }
    if poll.cancelled(at) {
        return None;
    }
    *state = current;
    Some(at)
}

#[cfg(test)]
#[path = "syntax_usda_stream_tests.rs"]
mod stream_tests;
