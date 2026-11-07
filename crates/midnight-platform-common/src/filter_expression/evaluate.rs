use super::{Field, Node, Predicate};
use midnight_core::{Entry, EntryKind};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Truth {
    False,
    True,
    Unknown,
}
impl From<bool> for Truth {
    fn from(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }
}
impl Truth {
    fn not(self) -> Self {
        match self {
            Self::False => Self::True,
            Self::True => Self::False,
            Self::Unknown => Self::Unknown,
        }
    }
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }
    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }
}
pub(super) fn matches(node: &Node, entry: &Entry) -> bool {
    evaluate(node, entry) == Truth::True
}
fn size(entry: &Entry) -> Option<u64> {
    (entry.kind == EntryKind::File)
        .then_some(entry.metadata.size)
        .flatten()
}

fn name(entry: &Entry) -> Option<&str> {
    match &entry.virtual_node {
        None => entry.name.to_str(),
        Some(node) => {
            // Archive names are opaque bytes without an encoding declaration.
            // A decoded or lossy caption cannot establish authoritative name text.
            std::str::from_utf8(&node.raw_name)
                .ok()
                .filter(|name| *name == node.display_name)
        }
    }
}

fn evaluate(node: &Node, entry: &Entry) -> Truth {
    match node {
        Node::And(left, right) => match evaluate(left, entry) {
            Truth::False => Truth::False,
            left => left.and(evaluate(right, entry)),
        },
        Node::Or(left, right) => match evaluate(left, entry) {
            Truth::True => Truth::True,
            left => left.or(evaluate(right, entry)),
        },
        Node::Not(inner) => evaluate(inner, entry).not(),
        Node::Predicate(predicate) => match predicate {
            Predicate::Name { pattern, negate } => name(entry)
                .map(|name| (pattern.is_match(name) != *negate).into())
                .unwrap_or(Truth::Unknown),
            Predicate::Size(comparison, expected) => size(entry)
                .map(|value| comparison.test(value, *expected).into())
                .unwrap_or(Truth::Unknown),
            Predicate::Modified(comparison, expected) => entry
                .metadata
                .modified
                .map(|value| {
                    comparison
                        .test(super::values::instant(value), *expected)
                        .into()
                })
                .unwrap_or(Truth::Unknown),
            Predicate::Kind { value, negate } => ((entry.kind == *value) != *negate).into(),
            Predicate::Attribute(mask) => entry
                .metadata
                .native_attributes
                .map(|value| (value & mask != 0).into())
                .unwrap_or(Truth::Unknown),
            Predicate::Presence { field, present } => {
                let known = match field {
                    Field::Name => name(entry).is_some(),
                    Field::Size => size(entry).is_some(),
                    Field::Modified => entry.metadata.modified.is_some(),
                    Field::Kind => true,
                    Field::Attributes => entry.metadata.native_attributes.is_some(),
                };
                (known == *present).into()
            }
        },
    }
}
