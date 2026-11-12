use super::{Comparison, Diagnostic, DiagnosticKind, SOURCE_BYTES, TOKENS};
use std::{iter::Peekable, ops::Range, str::CharIndices};

#[derive(Clone)]
pub(super) enum Kind {
    Word(String),
    String(String),
    Compare(Comparison),
    Regex,
    Left,
    Right,
}
#[derive(Clone)]
pub(super) struct Token {
    pub kind: Kind,
    pub span: Range<usize>,
}
pub(super) fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    if source.len() > SOURCE_BYTES {
        return Err(Diagnostic::new(
            0..source.len(),
            DiagnosticKind::Limit,
            "An expression may contain at most 4096 UTF-8 bytes.",
        ));
    }
    let mut chars = source.char_indices().peekable();
    let mut tokens = Vec::new();
    while let Some((start, ch)) = chars.next() {
        if ch.is_ascii_whitespace() {
            continue;
        }
        if tokens.len() == TOKENS {
            return Err(Diagnostic::new(
                start..start + ch.len_utf8(),
                DiagnosticKind::Limit,
                "An expression may contain at most 512 tokens.",
            ));
        }
        let kind = match ch {
            '(' => Kind::Left,
            ')' => Kind::Right,
            '"' => Kind::String(string(&mut chars, start, source.len())?),
            '=' if take(&mut chars, '~') => Kind::Regex,
            '=' => Kind::Compare(Comparison::Eq),
            '!' if take(&mut chars, '=') => Kind::Compare(Comparison::Ne),
            '<' => Kind::Compare(if take(&mut chars, '=') {
                Comparison::Le
            } else {
                Comparison::Lt
            }),
            '>' => Kind::Compare(if take(&mut chars, '=') {
                Comparison::Ge
            } else {
                Comparison::Gt
            }),
            c if c.is_ascii_alphanumeric() || c == '_' || c == '-' => {
                while chars
                    .peek()
                    .is_some_and(|(_, c)| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                {
                    chars.next();
                }
                let end = chars.peek().map_or(source.len(), |(i, _)| *i);
                Kind::Word(source[start..end].into())
            }
            _ => {
                return Err(Diagnostic::new(
                    start..start + ch.len_utf8(),
                    DiagnosticKind::Syntax,
                    "Unexpected character; fields and keywords use ASCII and strings use double quotes.",
                ));
            }
        };
        let end = chars.peek().map_or(source.len(), |(i, _)| *i);
        tokens.push(Token {
            kind,
            span: start..end,
        });
    }
    Ok(tokens)
}
fn take(chars: &mut Peekable<CharIndices<'_>>, expected: char) -> bool {
    if chars.peek().is_some_and(|(_, c)| *c == expected) {
        chars.next();
        true
    } else {
        false
    }
}
fn string(
    chars: &mut Peekable<CharIndices<'_>>,
    start: usize,
    end: usize,
) -> Result<String, Diagnostic> {
    let mut value = String::new();
    while let Some((offset, ch)) = chars.next() {
        match ch {
            '"' => return Ok(value),
            '\0' => {
                return Err(Diagnostic::new(
                    offset..offset + 1,
                    DiagnosticKind::Literal,
                    "A string cannot contain NUL.",
                ));
            }
            '\\' => {
                let Some((escaped, ch)) = chars.next() else {
                    break;
                };
                value.push(match ch {
                    '\\' => '\\',
                    '"' => '"',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    _ => {
                        return Err(Diagnostic::new(
                            offset..escaped + ch.len_utf8(),
                            DiagnosticKind::Literal,
                            "Unknown string escape; use \\\\, \\\", \\n, \\r or \\t.",
                        ));
                    }
                });
            }
            _ => value.push(ch),
        }
    }
    Err(Diagnostic::new(
        start..end,
        DiagnosticKind::Syntax,
        "Unterminated double-quoted string.",
    ))
}
