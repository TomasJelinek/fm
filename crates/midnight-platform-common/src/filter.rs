//! Bounded worker-side panel/name filter compilation; names are never converted lossily.
use midnight_core::{
    Entry, EntryKind, FilterDiagnostic, FilterDiagnosticKind, FilterMasks, FilterMode, PanelFilter,
};
use regex::{Regex, RegexBuilder};
use std::ffi::OsStr;

pub(crate) const REGEX_BYTES: usize = 2 * 1024 * 1024;

pub struct CompiledFilter {
    include: Option<Regex>,
    exclude: Option<Regex>,
}

impl CompiledFilter {
    pub fn new(filter: &PanelFilter) -> Result<Self, String> {
        FilterPlan::new(filter)?
            .compile(REGEX_BYTES)
            .map_err(String::from)
    }

    /// Invalid Unicode names remain visible. Their raw identity is never replaced.
    pub fn matches(&self, name: &OsStr) -> bool {
        let Some(name) = name.to_str() else {
            return true;
        };
        self.include
            .as_ref()
            .is_none_or(|regex| regex.is_match(name))
            && !self
                .exclude
                .as_ref()
                .is_some_and(|regex| regex.is_match(name))
    }
}

/// Why a prepared pattern did not compile into a matcher.
pub(crate) enum PatternError {
    /// An invalid pattern (or engine budget); the message says why.
    Invalid(String),
    /// A valid pattern whose matcher would exceed its byte budget.
    TooLarge(String),
}

impl From<PatternError> for String {
    fn from(error: PatternError) -> Self {
        match error {
            PatternError::Invalid(message) | PatternError::TooLarge(message) => message,
        }
    }
}

/// Bounded pattern preparation shared by ordinary filters and comparator budget planning.
/// Preparation parses the existing mask grammar without constructing any regex engine.
pub(crate) struct FilterPlan {
    include: Option<String>,
    exclude: Option<String>,
    case_sensitive: bool,
    dot_matches_new_line: bool,
}

impl FilterPlan {
    pub(crate) fn new(filter: &PanelFilter) -> Result<Self, String> {
        filter.validate_bounds().map_err(str::to_owned)?;
        if filter.mode == FilterMode::Expression {
            return Err("Metadata expressions require entry metadata; filename filters support Masks or Regex.".into());
        }
        if !filter.enabled {
            return Ok(Self {
                include: None,
                exclude: None,
                case_sensitive: filter.case_sensitive,
                dot_matches_new_line: filter.mode == FilterMode::Masks,
            });
        }
        let (include, exclude) = match filter.mode {
            FilterMode::Regex => (filter.expression.clone(), None),
            FilterMode::Masks => masks(&filter.expression)?,
            FilterMode::Expression => {
                return Err("Metadata expressions require entry metadata.".into());
            }
        };
        Ok(Self {
            include: Some(include),
            exclude,
            case_sensitive: filter.case_sensitive,
            dot_matches_new_line: filter.mode == FilterMode::Masks,
        })
    }

    pub(crate) fn engine_count(&self) -> usize {
        usize::from(self.include.is_some()) + usize::from(self.exclude.is_some())
    }

    /// Each retained engine receives this program ceiling AND this DFA ceiling.
    /// The caller accounts for both before constructing engines. Ordinary filters keep 2 MiB.
    pub(crate) fn compile(self, engine_bytes: usize) -> Result<CompiledFilter, PatternError> {
        if engine_bytes == 0 || engine_bytes > REGEX_BYTES {
            return Err(PatternError::Invalid(
                "Filename matcher engine budget must be between 1 byte and 2 MiB.".into(),
            ));
        }
        let compile = |pattern: &str| {
            RegexBuilder::new(pattern)
                .case_insensitive(!self.case_sensitive)
                .dot_matches_new_line(self.dot_matches_new_line)
                .size_limit(engine_bytes)
                .dfa_size_limit(engine_bytes)
                .nest_limit(64)
                .build()
                .map_err(|error| match error {
                    regex::Error::CompiledTooBig(_) => PatternError::TooLarge(error.to_string()),
                    _ => PatternError::Invalid(error.to_string()),
                })
        };
        Ok(CompiledFilter {
            include: self.include.as_deref().map(compile).transpose()?,
            exclude: self.exclude.as_deref().map(compile).transpose()?,
        })
    }
}

/// Panel filtering receives complete captured entries; legacy modes keep folders visible.
pub struct CompiledPanelFilter(PanelMatcher);

enum PanelMatcher {
    Disabled,
    Name(CompiledFilter),
    Expression(crate::filter_expression::CompiledExpression),
}

impl CompiledPanelFilter {
    pub fn new(filter: &PanelFilter) -> Result<Self, FilterDiagnostic> {
        filter.validate_bounds().map_err(|message| {
            FilterDiagnostic::new(
                0..filter.expression.len(),
                FilterDiagnosticKind::Limit,
                message,
            )
        })?;
        if filter.mode == FilterMode::Expression {
            // Accepted expression drafts are always typed, even when disabled.
            let expression = crate::filter_expression::CompiledExpression::new(
                &filter.expression,
                filter.case_sensitive,
            )?;
            return Ok(Self(if filter.enabled {
                PanelMatcher::Expression(expression)
            } else {
                PanelMatcher::Disabled
            }));
        }
        CompiledFilter::new(filter)
            .map(|names| Self(PanelMatcher::Name(names)))
            .map_err(|message| {
                FilterDiagnostic::new(
                    0..filter.expression.len(),
                    FilterDiagnosticKind::Pattern,
                    message,
                )
            })
    }
    pub fn matches_entry(&self, entry: &Entry) -> bool {
        match &self.0 {
            PanelMatcher::Disabled => true,
            PanelMatcher::Name(names) => {
                matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink)
                    || names.matches(&entry.name)
            }
            PanelMatcher::Expression(expression) => expression.matches(entry),
        }
    }
}

/// The include and exclude regexes of a Masks expression. The grammar is
/// `FilterMasks`, which the Select by mask dialog checks too; without include
/// masks every name is included.
fn masks(expression: &str) -> Result<(String, Option<String>), String> {
    let masks = FilterMasks::parse(expression).map_err(|error| error.message().to_owned())?;
    let group = |list: &[String]| {
        let mut alternatives: Vec<String> =
            list.iter().map(String::as_str).map(mask_regex).collect();
        if alternatives.is_empty() {
            alternatives.push(".*".into());
        }
        format!("\\A(?:{})\\z", alternatives.join("|"))
    };
    Ok((
        group(&masks.include),
        (!masks.exclude.is_empty()).then(|| group(&masks.exclude)),
    ))
}

/// `*` is any text and `?` one character; `*.*` is every name, extensionless too.
fn mask_regex(mask: &str) -> String {
    if mask == "*.*" {
        return ".*".into();
    }
    let mut regex = String::new();
    for ch in mask.chars() {
        match ch {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            _ => regex.push_str(&regex::escape(&ch.to_string())),
        }
    }
    regex
}
