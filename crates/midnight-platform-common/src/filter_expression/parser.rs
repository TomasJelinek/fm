use super::{
    Comparison, DEPTH, Diagnostic, DiagnosticKind, Field, NODES, Node, PATTERNS, Predicate,
    lexer::{self, Kind, Token},
    values,
};
use midnight_core::EntryKind;
use regex::RegexBuilder;
use std::ops::Range;

pub(super) fn compile(source: &str, case_sensitive: bool) -> Result<Node, Diagnostic> {
    let mut parser = Parser {
        tokens: lexer::lex(source)?,
        cursor: 0,
        end: source.len(),
        nodes: 0,
        patterns: 0,
        case_sensitive,
    };
    let root = parser.or(0)?;
    if parser.cursor != parser.tokens.len() {
        return Err(parser.error(DiagnosticKind::Syntax, "Unexpected token after expression."));
    }
    Ok(root)
}
struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    end: usize,
    nodes: usize,
    patterns: usize,
    case_sensitive: bool,
}
impl Parser {
    fn span(&self) -> Range<usize> {
        self.tokens
            .get(self.cursor)
            .map_or(self.end..self.end, |token| token.span.clone())
    }
    fn error(&self, kind: DiagnosticKind, message: &str) -> Diagnostic {
        Diagnostic::new(self.span(), kind, message)
    }
    fn word(&mut self, expected: &str) -> bool {
        if self
            .tokens
            .get(self.cursor)
            .is_some_and(|t| matches!(&t.kind, Kind::Word(word) if word == expected))
        {
            self.cursor += 1;
            true
        } else {
            false
        }
    }
    fn next(&mut self) -> Result<Token, Diagnostic> {
        let token =
            self.tokens.get(self.cursor).cloned().ok_or_else(|| {
                self.error(DiagnosticKind::Syntax, "Expected a predicate or value.")
            })?;
        self.cursor += 1;
        Ok(token)
    }
    fn node(&mut self, node: Node) -> Result<Node, Diagnostic> {
        self.nodes += 1;
        if self.nodes > NODES {
            Err(self.error(
                DiagnosticKind::Limit,
                "An expression may contain at most 256 nodes.",
            ))
        } else {
            Ok(node)
        }
    }
    fn or(&mut self, depth: usize) -> Result<Node, Diagnostic> {
        let mut left = self.and(depth)?;
        while self.word("or") {
            let right = self.and(depth)?;
            left = self.node(Node::Or(Box::new(left), Box::new(right)))?;
        }
        Ok(left)
    }
    fn and(&mut self, depth: usize) -> Result<Node, Diagnostic> {
        let mut left = self.unary(depth)?;
        while self.word("and") {
            let right = self.unary(depth)?;
            left = self.node(Node::And(Box::new(left), Box::new(right)))?;
        }
        Ok(left)
    }
    fn unary(&mut self, depth: usize) -> Result<Node, Diagnostic> {
        if depth > DEPTH {
            return Err(self.error(
                DiagnosticKind::Limit,
                "Expression nesting may not exceed 32.",
            ));
        }
        if self.word("not") {
            let inner = self.unary(depth + 1)?;
            return self.node(Node::Not(Box::new(inner)));
        }
        if self
            .tokens
            .get(self.cursor)
            .is_some_and(|t| matches!(t.kind, Kind::Left))
        {
            self.cursor += 1;
            let node = self.or(depth + 1)?;
            self.right()?;
            return Ok(node);
        }
        let predicate = self.predicate()?;
        self.node(Node::Predicate(predicate))
    }
    fn right(&mut self) -> Result<(), Diagnostic> {
        let token = self.next()?;
        if matches!(token.kind, Kind::Right) {
            Ok(())
        } else {
            Err(Diagnostic::new(
                token.span,
                DiagnosticKind::Syntax,
                "Expected closing parenthesis.",
            ))
        }
    }
    fn field(&mut self) -> Result<Field, Diagnostic> {
        let token = self.next()?;
        if let Kind::Word(word) = &token.kind
            && let Some(field) = Field::parse(word)
        {
            return Ok(field);
        }
        Err(Diagnostic::new(
            token.span,
            DiagnosticKind::Type,
            "Expected name, size, modified, kind or attributes.",
        ))
    }
    fn predicate(&mut self) -> Result<Predicate, Diagnostic> {
        let presence = if self.word("present") {
            Some(true)
        } else if self.word("missing") {
            Some(false)
        } else {
            None
        };
        if let Some(present) = presence {
            let token = self.next()?;
            if !matches!(token.kind, Kind::Left) {
                return Err(Diagnostic::new(
                    token.span,
                    DiagnosticKind::Syntax,
                    "Expected opening parenthesis.",
                ));
            }
            let field = self.field()?;
            self.right()?;
            return Ok(Predicate::Presence { field, present });
        }
        let field = self.field()?;
        let operator = self.next()?;
        match field {
            Field::Name => self.name(operator),
            Field::Attributes => {
                if !matches!(&operator.kind, Kind::Word(word) if word == "has") {
                    return Err(Diagnostic::new(
                        operator.span,
                        DiagnosticKind::Type,
                        "Attributes support 'has' with a reported native flag.",
                    ));
                }
                let token = self.next()?;
                let mask = if let Kind::Word(word) = &token.kind {
                    match word.as_str() {
                        "readonly" => Some(0x1),
                        "hidden" => Some(0x2),
                        "system" => Some(0x4),
                        "archive" => Some(0x20),
                        "temporary" => Some(0x100),
                        "sparse" => Some(0x200),
                        "compressed" => Some(0x800),
                        "offline" => Some(0x1000),
                        "encrypted" => Some(0x4000),
                        _ => None,
                    }
                } else {
                    None
                };
                mask.map(Predicate::Attribute).ok_or_else(|| {
                    Diagnostic::new(
                        token.span,
                        DiagnosticKind::Type,
                        "Unknown native attribute flag.",
                    )
                })
            }
            field => {
                let Kind::Compare(comparison) = operator.kind else {
                    return Err(Diagnostic::new(
                        operator.span,
                        DiagnosticKind::Type,
                        "This field requires a typed comparison operator.",
                    ));
                };
                let token = self.next()?;
                match (field, &token.kind) {
                    (Field::Size, Kind::Word(word)) => values::size(word).map(|value| Predicate::Size(comparison, value)).ok_or_else(|| Diagnostic::new(token.span, DiagnosticKind::Literal, "Use an unsigned byte size with optional B, kB/MB/GB/TB or KiB/MiB/GiB/TiB; overflow is not allowed.")),
                    (Field::Modified, Kind::String(value)) => values::timestamp(value).map(|value| Predicate::Modified(comparison, value)).ok_or_else(|| Diagnostic::new(token.span, DiagnosticKind::Literal, "Use a valid timestamp YYYY-MM-DDTHH:MM:SS[.fffffffff] with explicit Z or +HH:MM/-HH:MM; no local/date-only times.")),
                    (Field::Kind, Kind::Word(word)) if matches!(comparison, Comparison::Eq | Comparison::Ne) => {
                        let value = match word.as_str() { "file" => Some(EntryKind::File), "directory" => Some(EntryKind::Directory), "directory-link" => Some(EntryKind::DirectoryLink), "symlink" => Some(EntryKind::Symlink), "other" => Some(EntryKind::Other), _ => None };
                        value.map(|value| Predicate::Kind { value, negate: comparison == Comparison::Ne }).ok_or_else(|| Diagnostic::new(token.span, DiagnosticKind::Type, "Unknown entry kind."))
                    }
                    _ => Err(Diagnostic::new(token.span, DiagnosticKind::Type, "Value/operator does not have the field's declared type.")),
                }
            }
        }
    }
    fn name(&mut self, operator: Token) -> Result<Predicate, Diagnostic> {
        enum Pattern {
            Literal,
            Glob,
            Regex,
        }
        let (mode, negate) = match &operator.kind {
            Kind::Compare(Comparison::Eq) => (Pattern::Literal, false),
            Kind::Compare(Comparison::Ne) => (Pattern::Literal, true),
            Kind::Word(word) if word == "like" => (Pattern::Glob, false),
            Kind::Regex => (Pattern::Regex, false),
            _ => {
                return Err(Diagnostic::new(
                    operator.span,
                    DiagnosticKind::Type,
                    "Name supports =, !=, like or =~.",
                ));
            }
        };
        let token = self.next()?;
        let Kind::String(value) = token.kind else {
            return Err(Diagnostic::new(
                token.span,
                DiagnosticKind::Type,
                "Name predicates require a double-quoted string.",
            ));
        };
        if self.patterns == PATTERNS {
            return Err(Diagnostic::new(
                token.span,
                DiagnosticKind::Limit,
                "An expression may contain at most 16 name patterns.",
            ));
        }
        self.patterns += 1;
        let pattern = match mode {
            Pattern::Literal => format!("\\A{}\\z", regex::escape(&value)),
            Pattern::Glob => {
                let mut pattern = String::from("\\A");
                for ch in value.chars() {
                    match ch {
                        '*' => pattern.push_str(".*"),
                        '?' => pattern.push('.'),
                        _ => pattern.push_str(&regex::escape(&ch.to_string())),
                    }
                }
                pattern.push_str("\\z");
                pattern
            }
            Pattern::Regex => value,
        };
        // Per leaf, 64 KiB compiled automata plus 64 KiB DFA cache; at most 16 leaves.
        // This is a resource budget, not a claim that regex evaluation never allocates.
        let pattern = RegexBuilder::new(&pattern)
            .case_insensitive(!self.case_sensitive)
            .dot_matches_new_line(!matches!(mode, Pattern::Regex))
            .size_limit(64 * 1024)
            .dfa_size_limit(64 * 1024)
            .nest_limit(32)
            .build()
            .map_err(|error| {
                Diagnostic::new(token.span, DiagnosticKind::Pattern, error.to_string())
            })?;
        Ok(Predicate::Name { pattern, negate })
    }
}
