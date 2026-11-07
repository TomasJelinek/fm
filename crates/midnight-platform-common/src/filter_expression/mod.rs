//! Bounded typed predicates over captured listing metadata; no filesystem or script execution.
mod evaluate;
mod lexer;
mod parser;
mod values;

use midnight_core::{Entry, EntryKind};
pub use midnight_core::{FilterDiagnostic as Diagnostic, FilterDiagnosticKind as DiagnosticKind};
use regex::Regex;

const SOURCE_BYTES: usize = midnight_core::MAX_FILTER_BYTES;
const TOKENS: usize = 512;
const NODES: usize = 256;
const DEPTH: usize = 32;
const PATTERNS: usize = 16;

pub struct CompiledExpression(Node);
impl CompiledExpression {
    pub fn new(source: &str, case_sensitive: bool) -> Result<Self, Diagnostic> {
        parser::compile(source, case_sensitive).map(Self)
    }
    /// Evaluates captured metadata only. Regex uses bounded reusable worker scratch.
    pub fn matches(&self, entry: &Entry) -> bool {
        evaluate::matches(&self.0, entry)
    }
}

enum Node {
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    Not(Box<Node>),
    Predicate(Predicate),
}
enum Predicate {
    Name { pattern: Regex, negate: bool },
    Size(Comparison, u64),
    Modified(Comparison, i128),
    Kind { value: EntryKind, negate: bool },
    Attribute(u32),
    Presence { field: Field, present: bool },
}
#[derive(Clone, Copy)]
enum Field {
    Name,
    Size,
    Modified,
    Kind,
    Attributes,
}
impl Field {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "name" => Self::Name,
            "size" => Self::Size,
            "modified" => Self::Modified,
            "kind" => Self::Kind,
            "attributes" => Self::Attributes,
            _ => return None,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Comparison {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
impl Comparison {
    fn test<T: Ord>(self, value: T, expected: T) -> bool {
        match self {
            Self::Eq => value == expected,
            Self::Ne => value != expected,
            Self::Lt => value < expected,
            Self::Le => value <= expected,
            Self::Gt => value > expected,
            Self::Ge => value >= expected,
        }
    }
}
