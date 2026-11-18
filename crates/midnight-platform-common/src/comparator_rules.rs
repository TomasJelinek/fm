//! Worker-side comparator compilation and ordered matching of original native basenames.
use crate::filter::{CompiledFilter, FilterPlan, REGEX_BYTES};
use midnight_core::{
    CompiledFileComparatorPolicy, FileComparatorDiagnostic, FileComparatorField,
    FileComparatorPreferences, MAX_FILE_COMPARATOR_MATCHER_BYTES,
};
use std::{path::PathBuf, sync::Arc};

struct CompiledComparators {
    preferences: Arc<FileComparatorPreferences>,
    patterns: Vec<CompiledFilter>,
    matcher_budget_bytes: usize,
}

/// Compile every rule, including disabled rules, before accepting a changed configuration.
///
/// The returned immutable policy retains the EXACT submitted raw Arc. The sole preferences
/// worker owns compilation, candidate rejection and eventual destruction; sessions only pin
/// already accepted policies. This compiler has no current-policy store or history cache.
///
/// Each retained include/exclude regex receives equal program and DFA ceilings, each at most
/// the existing 2 MiB default. Their checked sum is at most 16 MiB. These are configured engine
/// ceilings, not an RSS bound: raw settings, plans, regex metadata, allocator overhead and the
/// one active compiler's transient allocations are additional.
pub fn compile(
    preferences: Arc<FileComparatorPreferences>,
) -> Result<Arc<dyn CompiledFileComparatorPolicy>, FileComparatorDiagnostic> {
    preferences.validate()?;
    let mut plans = Vec::with_capacity(preferences.rules.len());
    let mut engines = 0usize;
    for (index, rule) in preferences.rules.iter().enumerate() {
        // filter() deliberately sets enabled=true: disabled patterns must validate too.
        let plan =
            FilterPlan::new(&rule.filter()).map_err(|message| pattern_error(index, message))?;
        engines = engines
            .checked_add(plan.engine_count())
            .ok_or_else(budget_error)?;
        plans.push(plan);
    }

    let (engine_bytes, matcher_budget_bytes) = engine_budget(engines)?;
    let mut patterns = Vec::with_capacity(plans.len());
    for (index, plan) in plans.into_iter().enumerate() {
        patterns.push(plan.compile(engine_bytes).map_err(|message| {
            pattern_error(
                index,
                format!(
                    "{message} Comparator matcher limits are {engine_bytes} bytes each for \
                     the program and DFA of every include/exclude engine (16 MiB per rule set)."
                ),
            )
        })?);
    }
    Ok(Arc::new(CompiledComparators {
        preferences,
        patterns,
        matcher_budget_bytes,
    }))
}

/// Standalone full semantic validation. Run and drop the temporary policy on a backend worker.
/// Prepared persistence uses the already compiled policy's proof instead of compiling again.
pub fn validate(preferences: &FileComparatorPreferences) -> Result<(), FileComparatorDiagnostic> {
    // Reject unbounded caller data before duplicating the bounded raw configuration.
    preferences.validate()?;
    compile(Arc::new(preferences.clone())).map(drop)
}

fn engine_budget(engines: usize) -> Result<(usize, usize), FileComparatorDiagnostic> {
    if engines == 0 {
        return Ok((REGEX_BYTES, 0));
    }
    let ceilings = engines.checked_mul(2).ok_or_else(budget_error)?;
    let bytes = (MAX_FILE_COMPARATOR_MATCHER_BYTES / ceilings).min(REGEX_BYTES);
    if bytes == 0 {
        return Err(budget_error());
    }
    let total = ceilings.checked_mul(bytes).ok_or_else(budget_error)?;
    if total > MAX_FILE_COMPARATOR_MATCHER_BYTES {
        return Err(budget_error());
    }
    Ok((bytes, total))
}

impl CompiledFileComparatorPolicy for CompiledComparators {
    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.preferences
    }

    fn select(&self, original_paths: &[PathBuf; 2]) -> Option<usize> {
        let left = original_paths[0].file_name()?;
        let right = original_paths[1].file_name()?;
        // CompiledFilter intentionally keeps raw panel names visible. Comparator authorization
        // must first require BOTH original names to be losslessly representable as Unicode.
        left.to_str()?;
        right.to_str()?;
        self.preferences
            .rules
            .iter()
            .zip(&self.patterns)
            .position(|(rule, pattern)| {
                rule.enabled && pattern.matches(left) && pattern.matches(right)
            })
    }

    fn matcher_budget_bytes(&self) -> usize {
        self.matcher_budget_bytes
    }
}

fn pattern_error(index: usize, message: String) -> FileComparatorDiagnostic {
    FileComparatorDiagnostic {
        rule: Some(index),
        field: FileComparatorField::Expression,
        message,
    }
}

fn budget_error() -> FileComparatorDiagnostic {
    FileComparatorDiagnostic {
        rule: None,
        field: FileComparatorField::Rules,
        message: "Comparator matchers exceed the 16 MiB configured engine budget.".into(),
    }
}
