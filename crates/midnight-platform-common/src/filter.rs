//! Bounded worker-side panel/name filter compilation; names are never converted lossily.
use midnight_core::{
    Entry, EntryKind, FilterDiagnostic, FilterDiagnosticKind, FilterMode, PanelFilter,
};
use regex::{Regex, RegexBuilder};
use std::ffi::OsStr;

const REGEX_BYTES: usize = 2 * 1024 * 1024;
const MAX_MASKS: usize = 64;

pub struct CompiledFilter {
    include: Option<Regex>,
    exclude: Option<Regex>,
}

impl CompiledFilter {
    pub fn new(filter: &PanelFilter) -> Result<Self, String> {
        filter.validate_bounds().map_err(str::to_owned)?;
        if filter.mode == FilterMode::Expression {
            return Err("Metadata expressions require entry metadata; filename filters support Masks or Regex.".into());
        }
        if !filter.enabled {
            return Ok(Self {
                include: None,
                exclude: None,
            });
        }
        let compile = |pattern: &str| {
            RegexBuilder::new(pattern)
                .case_insensitive(!filter.case_sensitive)
                .dot_matches_new_line(filter.mode == FilterMode::Masks)
                .size_limit(REGEX_BYTES)
                .dfa_size_limit(REGEX_BYTES)
                .nest_limit(64)
                .build()
                .map_err(|error| error.to_string())
        };
        match filter.mode {
            FilterMode::Regex => Ok(Self {
                include: Some(compile(&filter.expression)?),
                exclude: None,
            }),
            FilterMode::Masks => {
                let (include, exclude) = masks(&filter.expression)?;
                Ok(Self {
                    include: Some(compile(&include)?),
                    exclude: exclude.as_deref().map(compile).transpose()?,
                })
            }
            FilterMode::Expression => Err("Metadata expressions require entry metadata.".into()),
        }
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

fn masks(expression: &str) -> Result<(String, Option<String>), String> {
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
                        return Err("Use at most one | before excluded masks.".into());
                    }
                    excluded = true;
                }
            }
            _ => current.push(ch),
        }
    }
    push_mask(&mut groups[usize::from(excluded)], &mut current);
    if groups.iter().map(Vec::len).sum::<usize>() > MAX_MASKS {
        return Err("A panel filter may contain at most 64 masks.".into());
    }
    if excluded && groups[1].is_empty() {
        return Err("Add at least one excluded mask after |.".into());
    }
    if groups[0].is_empty() {
        groups[0].push(".*".into());
    }
    let expression = |group: &[String]| format!("\\A(?:{})\\z", group.join("|"));
    Ok((
        expression(&groups[0]),
        excluded.then(|| expression(&groups[1])),
    ))
}

fn push_mask(group: &mut Vec<String>, current: &mut String) {
    let mask = current.trim();
    if !mask.is_empty() {
        if mask == "*.*" {
            group.push(".*".into());
        } else {
            let mut regex = String::new();
            for ch in mask.chars() {
                match ch {
                    '*' => regex.push_str(".*"),
                    '?' => regex.push('.'),
                    _ => regex.push_str(&regex::escape(&ch.to_string())),
                }
            }
            group.push(regex);
        }
    }
    current.clear();
}
