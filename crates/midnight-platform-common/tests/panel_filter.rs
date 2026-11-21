use midnight_core::{FilterMode, MaskSelection, MaskSelectionPreferences, PanelFilter};
use midnight_platform_common::filter::{CompiledFilter, MASK_TOO_LARGE, validate_selection_mask};
use std::ffi::OsStr;

fn masks(expression: &str) -> PanelFilter {
    PanelFilter {
        enabled: true,
        expression: expression.into(),
        ..Default::default()
    }
}

#[test]
fn masks_include_exclude_escape_and_extensionless_names() {
    let all = CompiledFilter::new(&masks("*.*")).unwrap();
    assert!(all.matches(OsStr::new("README")));
    let selected =
        CompiledFilter::new(&masks("*.txt;report.?s;semi;;colon|secret*;*.bak")).unwrap();
    for name in ["note.TXT", "report.rs", "semi;colon"] {
        assert!(selected.matches(OsStr::new(name)), "{name}");
    }
    for name in ["secret.txt", "data.bak", "report.long", "other"] {
        assert!(!selected.matches(OsStr::new(name)), "{name}");
    }
    let exclude_only = CompiledFilter::new(&masks("|*.tmp")).unwrap();
    assert!(exclude_only.matches(OsStr::new("README")));
    assert!(!exclude_only.matches(OsStr::new("cache.tmp")));
}

#[test]
fn regex_and_explicit_case_policy_are_distinct_from_masks() {
    let mut filter = masks("^文[0-9]+\\.txt$");
    filter.mode = FilterMode::Regex;
    filter.case_sensitive = true;
    let compiled = CompiledFilter::new(&filter).unwrap();
    assert!(compiled.matches(OsStr::new("文42.txt")));
    assert!(!compiled.matches(OsStr::new("文42.TXT")));
    filter.case_sensitive = false;
    assert!(
        CompiledFilter::new(&filter)
            .unwrap()
            .matches(OsStr::new("文42.TXT"))
    );
    filter.expression = "^a.b$".into();
    assert!(
        !CompiledFilter::new(&filter)
            .unwrap()
            .matches(OsStr::new("a\nb")),
        "Rust regex dot excludes line breaks unless the user enables dot-all"
    );
    filter.expression = "(?s)^a.b$".into();
    assert!(
        CompiledFilter::new(&filter)
            .unwrap()
            .matches(OsStr::new("a\nb"))
    );
    assert!(
        CompiledFilter::new(&masks("a?b"))
            .unwrap()
            .matches(OsStr::new("a\nb"))
    );
}

#[test]
fn invalid_and_excessive_expressions_are_rejected_without_truncation() {
    for expression in ["*.txt|", "a|b|c"] {
        assert!(CompiledFilter::new(&masks(expression)).is_err());
    }
    let mut regex = masks("[");
    regex.mode = FilterMode::Regex;
    assert!(CompiledFilter::new(&regex).is_err());
    regex.expression = "a".repeat(midnight_core::MAX_FILTER_BYTES + 1);
    assert!(CompiledFilter::new(&regex).is_err());
    regex.expression = "(?:a{1000000}){1000000}".into();
    assert!(CompiledFilter::new(&regex).is_err());
}

#[test]
fn the_select_by_mask_dialog_and_the_mask_compiler_agree_up_to_the_largest_masks() {
    // The dialog applies and remembers a mask only once the backend's check
    // (`validate_selection_mask`, the dialog's grammar check and the compiler) has
    // accepted it, so an accepted mask must compile on the selection worker and a
    // refused one must not, also at the size bound, where wildcards and case
    // variants make the largest matchers.
    let longest = |unit: &str| unit.repeat(midnight_core::MAX_FILTER_BYTES / unit.len());
    let mut patterns: Vec<String> = [
        "*",
        "*.*",
        "*.txt;*.MD|draft*",
        "|*.tmp",
        "semi;;colon",
        ";",
        "文档*.TXT;é?",
        "a|b|c",
        "*.txt|",
        "*.txt| ;",
        "||",
        "|",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    patterns.extend(["*", "?", "*?", "k", "θ", "ǅ"].map(longest));
    patterns.push(vec!["?".repeat(63); 64].join(";"));
    patterns.push(vec!["?"; 65].join(";"));
    for pattern in patterns {
        let accepted = validate_selection_mask(&pattern);
        let mask = MaskSelection {
            pattern,
            select: true,
            folders: false,
        };
        let compiled = CompiledFilter::new(&mask.filter());
        assert_eq!(
            accepted.is_ok(),
            compiled.is_ok(),
            "{:?}: dialog {accepted:?}, compiler {:?}",
            mask.pattern,
            compiled.err()
        );
    }
}

#[test]
fn a_mask_of_thousands_of_stars_compiles_like_one_star() {
    let mask = MaskSelection {
        pattern: "*".repeat(4096),
        select: true,
        folders: false,
    };
    let compiled = CompiledFilter::new(&mask.filter());
    assert!(compiled.is_ok(), "{:?}", compiled.err());
}

#[test]
fn the_backend_check_refuses_a_mask_over_the_matcher_budget_with_a_reason() {
    let large = "?".repeat(midnight_core::MAX_FILTER_BYTES);
    let choices = MaskSelectionPreferences {
        pattern: large.clone(),
        folders: false,
    };
    assert_eq!(choices.validate_mask(), Ok(()), "the grammar allows it");
    assert_eq!(
        validate_selection_mask(&large),
        Err(MASK_TOO_LARGE.to_owned()),
        "its matcher does not fit"
    );
    // The dialog's own rules keep their texts.
    for (pattern, reason) in [
        (
            vec!["*.x"; 65].join(";"),
            "A selection mask may contain at most 64 masks.",
        ),
        (
            "*.txt\n".to_owned(),
            "A selection mask cannot contain control characters.",
        ),
        (
            "*.txt|".to_owned(),
            "Add at least one excluded mask after |.",
        ),
    ] {
        assert_eq!(
            validate_selection_mask(&pattern),
            Err(reason.to_owned()),
            "{pattern:?}"
        );
    }
    assert_eq!(validate_selection_mask(&"*".repeat(4096)), Ok(()));
    assert_eq!(validate_selection_mask("文档*.TXT;é?|draft*"), Ok(()));
}
