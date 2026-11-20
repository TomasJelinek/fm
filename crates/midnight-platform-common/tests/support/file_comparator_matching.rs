use midnight_core::{
    CompiledFileComparatorPolicy, EditorPreferences, EditorRule, FileComparatorField,
    FileComparatorPreferences, FileComparatorRule, FileComparatorTarget, FilterMode, PanelFilter,
};
use midnight_platform_common::{comparator_rules, editor_rules, filter::CompiledFilter};
use std::{ffi::OsStr, path::PathBuf, sync::Arc};

#[cfg(unix)]
#[path = "file_comparator_matching_unix.rs"]
mod unix;
#[cfg(windows)]
#[path = "file_comparator_matching_windows.rs"]
mod windows;

fn rule(expression: &str, target: FileComparatorTarget) -> FileComparatorRule {
    FileComparatorRule {
        enabled: true,
        mode: FilterMode::Masks,
        expression: expression.into(),
        case_sensitive: false,
        target,
    }
}

fn compile(rules: Vec<FileComparatorRule>) -> Arc<dyn CompiledFileComparatorPolicy> {
    comparator_rules::compile(Arc::new(FileComparatorPreferences { rules })).unwrap()
}

fn pair(left: &str, right: &str) -> [PathBuf; 2] {
    [PathBuf::from(left), PathBuf::from(right)]
}

#[test]
fn one_pattern_matches_both_different_original_basenames() {
    let policy = compile(vec![rule("*.c;*.cpp", FileComparatorTarget::BuiltInText)]);
    assert_eq!(policy.select(&pair("before.c", "after.cpp")), Some(0));
    assert_eq!(
        policy.select(&pair("old/before.cpp", "new/after.c")),
        Some(0)
    );
    assert_eq!(policy.select(&pair("before.c", "after.h")), None);
    assert_eq!(policy.select(&pair("before.h", "after.cpp")), None);
}

#[test]
fn first_enabled_match_wins_and_explicit_automatic_stops() {
    let mut disabled = rule("*", FileComparatorTarget::BuiltInImages);
    disabled.enabled = false;
    let policy = compile(vec![
        disabled,
        rule("secret*", FileComparatorTarget::Automatic),
        rule("*", FileComparatorTarget::BuiltInText),
    ]);
    assert_eq!(
        policy.select(&pair("secret-before", "secret-after")),
        Some(1)
    );
    assert_eq!(policy.select(&pair("secret-before", "ordinary")), Some(2));
    assert_eq!(policy.select(&pair("one", "two")), Some(2));
}

#[test]
fn separate_filetype_rules_do_not_choose_a_mixed_pair() {
    let policy = compile(vec![
        rule("*.txt", FileComparatorTarget::BuiltInText),
        rule("*.png", FileComparatorTarget::BuiltInImages),
    ]);
    assert_eq!(policy.select(&pair("one.txt", "two.png")), None);
    assert_eq!(policy.select(&pair("one.png", "two.txt")), None);
    assert_eq!(policy.select(&pair("one.bin", "two.bin")), None);
    assert_eq!(compile(Vec::new()).select(&pair("one", "two")), None);
}

#[test]
fn shared_mask_exclusions_and_escaped_separators_apply_to_each_side() {
    let policy = compile(vec![rule(
        "*.txt;semi;;colon.dat|secret*",
        FileComparatorTarget::BuiltInText,
    )]);
    assert_eq!(
        policy.select(&pair("semi;colon.dat", "readme.txt")),
        Some(0)
    );
    assert_eq!(policy.select(&pair("secret.txt", "readme.txt")), None);
    assert_eq!(policy.select(&pair("readme.txt", "secret.txt")), None);
    assert_eq!(policy.select(&pair("semi", "colon.dat")), None);
    let wildcard = compile(vec![rule("*.*", FileComparatorTarget::Automatic)]);
    assert_eq!(wildcard.select(&pair("extensionless", "a\nb")), Some(0));
}

#[test]
fn case_and_regex_semantics_are_shared_with_filename_filters() {
    let insensitive = compile(vec![rule("*.TXT", FileComparatorTarget::BuiltInText)]);
    assert_eq!(insensitive.select(&pair("one.txt", "two.TxT")), Some(0));
    let mut sensitive = rule("*.TXT", FileComparatorTarget::BuiltInText);
    sensitive.case_sensitive = true;
    assert_eq!(
        compile(vec![sensitive]).select(&pair("one.TXT", "two.txt")),
        None
    );
    let mut regex = rule(r"(?i)\.(c|cpp)$", FileComparatorTarget::BuiltInText);
    regex.mode = FilterMode::Regex;
    regex.case_sensitive = true;
    let policy = compile(vec![regex]);
    assert_eq!(policy.select(&pair("one.C", "two.cpp")), Some(0));
    assert_eq!(policy.select(&pair("one.c.bak", "two.cpp")), None);
}

#[test]
fn unicode_names_match_without_normalizing_original_spelling() {
    let policy = compile(vec![rule("*.txt", FileComparatorTarget::BuiltInText)]);
    assert_eq!(
        policy.select(&pair("Žluťoučký 文档 😀.txt", "مرحبا שלום.txt")),
        Some(0)
    );
    let exact = compile(vec![rule("é.txt", FileComparatorTarget::Automatic)]);
    assert_eq!(exact.select(&pair("é.txt", "e\u{301}.txt")), None);
}

#[test]
fn missing_basename_uses_automatic() {
    let policy = compile(vec![rule("*", FileComparatorTarget::BuiltInText)]);
    assert_eq!(policy.select(&pair("", "two.txt")), None);
    assert_eq!(policy.select(&pair("one.txt", "")), None);
}

#[test]
fn disabled_invalid_regex_reports_its_submitted_row_and_expression_field() {
    let mut invalid = rule("[", FileComparatorTarget::BuiltInText);
    invalid.mode = FilterMode::Regex;
    invalid.enabled = false;
    let raw = FileComparatorPreferences {
        rules: vec![rule("*", FileComparatorTarget::Automatic), invalid],
    };
    let error = comparator_rules::validate(&raw).unwrap_err();
    assert_eq!(error.rule, Some(1));
    assert_eq!(error.field, FileComparatorField::Expression);
    // One sentence naming the problem, without the matcher size limits.
    assert_eq!(
        error.message,
        "Invalid regular expression: unclosed character class."
    );
    assert!(comparator_rules::compile(Arc::new(raw)).is_err());
}

#[test]
fn malformed_mask_exclusion_reports_the_row() {
    let raw = FileComparatorPreferences {
        rules: vec![rule("*.txt|", FileComparatorTarget::BuiltInText)],
    };
    let error = comparator_rules::validate(&raw).unwrap_err();
    assert_eq!(error.rule, Some(0));
    assert_eq!(error.field, FileComparatorField::Expression);
}

#[test]
fn sixty_four_include_exclude_rules_fit_combined_sixteen_mib_ceilings() {
    let policy = compile(vec![
        rule("*.txt|secret*", FileComparatorTarget::BuiltInText);
        64
    ]);
    assert_eq!(policy.matcher_budget_bytes(), 16 * 1024 * 1024);
    assert_eq!(policy.select(&pair("one.txt", "two.txt")), Some(0));
    assert_eq!(policy.select(&pair("one.txt", "secret.txt")), None);
}

#[test]
fn reported_budget_counts_both_limits_per_engine_and_caps_each_at_two_mib() {
    assert_eq!(compile(Vec::new()).matcher_budget_bytes(), 0);
    let one = compile(vec![rule("*.txt", FileComparatorTarget::Automatic)]);
    assert_eq!(one.matcher_budget_bytes(), 4 * 1024 * 1024);
    let two = compile(vec![rule("*.txt|secret*", FileComparatorTarget::Automatic)]);
    assert_eq!(two.matcher_budget_bytes(), 8 * 1024 * 1024);
    let three = compile(vec![rule("*", FileComparatorTarget::Automatic); 3]);
    assert_eq!(three.matcher_budget_bytes(), 12 * 1024 * 1024);
    let five = compile(vec![rule("*", FileComparatorTarget::Automatic); 5]);
    assert_eq!(five.matcher_budget_bytes(), 16_777_210);
}

#[test]
fn disabled_rules_are_also_in_the_retained_budget() {
    let mut disabled = rule("*.txt|secret*", FileComparatorTarget::BuiltInText);
    disabled.enabled = false;
    let policy = compile(vec![disabled; 64]);
    assert_eq!(policy.matcher_budget_bytes(), 16 * 1024 * 1024);
    assert_eq!(policy.select(&pair("one.txt", "two.txt")), None);
}

#[test]
fn oversized_regex_is_rejected_with_a_row_and_field_diagnostic() {
    let mut oversized = rule(r"(?:a{1000}){1000}", FileComparatorTarget::BuiltInText);
    oversized.mode = FilterMode::Regex;
    let error = comparator_rules::validate(&FileComparatorPreferences {
        rules: vec![oversized],
    })
    .unwrap_err();
    assert_eq!(error.rule, Some(0));
    assert_eq!(error.field, FileComparatorField::Expression);
    // Only a matcher that is too large explains the size limits.
    assert!(
        error.message.contains("Comparator matcher limits are"),
        "{}",
        error.message
    );
}

#[test]
fn comparator_aggregate_budget_does_not_reduce_panel_or_editor_defaults() {
    let expression = "a{10000}";
    let panel = PanelFilter {
        enabled: true,
        mode: FilterMode::Regex,
        expression: expression.into(),
        case_sensitive: true,
    };
    let ordinary = CompiledFilter::new(&panel).unwrap();
    assert!(ordinary.matches(OsStr::new(&"a".repeat(10_000))));
    editor_rules::validate(&EditorPreferences {
        rules: vec![EditorRule {
            mode: FilterMode::Regex,
            expression: expression.into(),
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap();
    let mut large = rule(expression, FileComparatorTarget::Automatic);
    large.mode = FilterMode::Regex;
    let raw = FileComparatorPreferences {
        rules: vec![large; 64],
    };
    assert!(comparator_rules::validate(&raw).is_err());
    assert!(CompiledFilter::new(&panel).is_ok());
}

#[test]
fn disabled_panel_filter_keeps_its_existing_invalid_regex_bypass() {
    let panel = PanelFilter {
        enabled: false,
        mode: FilterMode::Regex,
        expression: "[".into(),
        case_sensitive: true,
    };
    let ordinary = CompiledFilter::new(&panel).unwrap();
    assert!(ordinary.matches(OsStr::new("anything.txt")));
}

#[test]
fn selection_indexes_the_exact_retained_raw_arc() {
    let raw = Arc::new(FileComparatorPreferences {
        rules: vec![
            rule("*.png", FileComparatorTarget::BuiltInImages),
            rule("*.txt", FileComparatorTarget::BuiltInText),
        ],
    });
    let policy = comparator_rules::compile(raw.clone()).unwrap();
    assert!(Arc::ptr_eq(policy.preferences(), &raw));
    let index = policy.select(&pair("one.txt", "two.txt")).unwrap();
    assert_eq!(index, 1);
    assert_eq!(
        policy.preferences().rules[index].target,
        FileComparatorTarget::BuiltInText
    );
}
