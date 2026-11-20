//! Saved per-panel name/metadata filtering; compilation and matching belong to workers.
use crate::PaneId;
use std::ops::Range;

pub const MAX_FILTER_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterMode {
    #[default]
    Masks,
    Regex,
    Expression,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterDiagnosticKind {
    Syntax,
    Type,
    Limit,
    Literal,
    Pattern,
}

/// UTF-8 byte range in the submitted expression, distinct from native names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterDiagnostic {
    pub span: Range<usize>,
    pub kind: FilterDiagnosticKind,
    pub message: String,
}

impl FilterDiagnostic {
    pub fn new(span: Range<usize>, kind: FilterDiagnosticKind, message: impl Into<String>) -> Self {
        Self {
            span,
            kind,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelFilter {
    pub enabled: bool,
    pub mode: FilterMode,
    pub expression: String,
    pub case_sensitive: bool,
}

impl Default for PanelFilter {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: FilterMode::Masks,
            expression: "*.*".into(),
            case_sensitive: false,
        }
    }
}

impl PanelFilter {
    /// Syntax and compiled-pattern budgets are checked by the backend compiler.
    pub fn validate_bounds(&self) -> Result<(), &'static str> {
        if self.expression.len() > MAX_FILTER_BYTES {
            Err("A panel filter may contain at most 4096 UTF-8 bytes.")
        } else if self.expression.contains('\0') {
            Err("A panel filter cannot contain NUL.")
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
pub struct FilterValidationRequest {
    pub id: u64,
    pub pane: PaneId,
    pub filter: PanelFilter,
}

/// Most masks one Masks expression may hold, excluded masks included.
pub const MAX_FILTER_MASKS: usize = 64;

/// Why a Masks expression does not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskSyntaxError {
    /// A second `|`: only one starts the excluded masks.
    SecondExclusion,
    /// A `|` without an excluded mask after it.
    MissingExclusion,
    /// More than `MAX_FILTER_MASKS` masks.
    TooManyMasks,
}

impl MaskSyntaxError {
    /// The panel filter's fixed text for this error.
    pub fn message(self) -> &'static str {
        match self {
            Self::SecondExclusion => "Use at most one | before excluded masks.",
            Self::MissingExclusion => "Add at least one excluded mask after |.",
            Self::TooManyMasks => "A panel filter may contain at most 64 masks.",
        }
    }
}

/// A Masks expression split by its grammar, the one rule set of every Masks user
/// (panel filters, Find, Select by mask): `;` separates masks, `;;` is a literal
/// semicolon, one `|` starts the excluded masks, and spaces around a mask are not
/// part of it. Only the structure is checked here; workers compile the masks' `*`
/// and `?` into bounded matchers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilterMasks {
    /// A name must match one of these; none admits every name.
    pub include: Vec<String>,
    /// A name that matches one of these is left out. Not empty exactly when the
    /// expression has its `|`.
    pub exclude: Vec<String>,
}

impl FilterMasks {
    /// Linear in the expression's length; callers bound that length first.
    pub fn parse(expression: &str) -> Result<Self, MaskSyntaxError> {
        let mut groups = [Vec::new(), Vec::new()];
        let mut excluded = false;
        let mut current = String::new();
        let mut chars = expression.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                ';' if chars.peek() == Some(&';') => {
                    chars.next();
                    current.push(';');
                }
                ';' | '|' => {
                    push_mask(&mut groups[usize::from(excluded)], &mut current);
                    if ch == '|' {
                        if excluded {
                            return Err(MaskSyntaxError::SecondExclusion);
                        }
                        excluded = true;
                    }
                }
                _ => current.push(ch),
            }
        }
        push_mask(&mut groups[usize::from(excluded)], &mut current);
        if groups.iter().map(Vec::len).sum::<usize>() > MAX_FILTER_MASKS {
            return Err(MaskSyntaxError::TooManyMasks);
        }
        let [include, exclude] = groups;
        if excluded && exclude.is_empty() {
            return Err(MaskSyntaxError::MissingExclusion);
        }
        Ok(Self { include, exclude })
    }
}

/// Keep the finished mask without its surrounding spaces, if anything is left.
fn push_mask(group: &mut Vec<String>, current: &mut String) {
    let mask = current.trim();
    if !mask.is_empty() {
        group.push(mask.to_owned());
    }
    current.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masks(include: &[&str], exclude: &[&str]) -> Result<FilterMasks, MaskSyntaxError> {
        let owned = |group: &[&str]| -> Vec<String> {
            group.iter().map(|mask| (*mask).to_owned()).collect()
        };
        Ok(FilterMasks {
            include: owned(include),
            exclude: owned(exclude),
        })
    }

    #[test]
    fn masks_split_at_semicolons_and_one_bar_and_keep_literal_semicolons() {
        for (expression, expected) in [
            ("*.txt", masks(&["*.txt"], &[])),
            (
                " *.txt ; report.?s;semi;;colon |secret* ;*.bak",
                masks(&["*.txt", "report.?s", "semi;colon"], &["secret*", "*.bak"]),
            ),
            ("|*.tmp", masks(&[], &["*.tmp"])),
            ("", masks(&[], &[])),
            (" ; ;", masks(&[], &[])),
            (";;;a", masks(&[";", "a"], &[])),
            ("文档*.TXT;é?", masks(&["文档*.TXT", "é?"], &[])),
        ] {
            assert_eq!(FilterMasks::parse(expression), expected, "{expression:?}");
        }
        let most = vec!["*.x"; MAX_FILTER_MASKS].join(";");
        assert_eq!(FilterMasks::parse(&most).unwrap().include.len(), 64);
    }

    #[test]
    fn a_second_bar_an_empty_exclusion_and_too_many_masks_are_errors() {
        for (expression, error) in [
            ("a|b|c", MaskSyntaxError::SecondExclusion),
            ("*.txt|*.md|", MaskSyntaxError::SecondExclusion),
            ("||", MaskSyntaxError::SecondExclusion),
            ("*.txt|", MaskSyntaxError::MissingExclusion),
            ("*.txt| ; ", MaskSyntaxError::MissingExclusion),
            ("|", MaskSyntaxError::MissingExclusion),
        ] {
            assert_eq!(FilterMasks::parse(expression), Err(error), "{expression:?}");
        }
        let included = vec!["*.x"; MAX_FILTER_MASKS + 1].join(";");
        let split = format!("{}|*.y", vec!["*.x"; MAX_FILTER_MASKS].join(";"));
        for expression in [included, split] {
            assert_eq!(
                FilterMasks::parse(&expression),
                Err(MaskSyntaxError::TooManyMasks)
            );
        }
        assert_eq!(
            MaskSyntaxError::SecondExclusion.message(),
            "Use at most one | before excluded masks."
        );
        assert_eq!(
            MaskSyntaxError::MissingExclusion.message(),
            "Add at least one excluded mask after |."
        );
        assert_eq!(
            MaskSyntaxError::TooManyMasks.message(),
            "A panel filter may contain at most 64 masks."
        );
    }
}
