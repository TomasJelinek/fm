use super::*;

// Historical five-rule preset: fixture for lossless preference migration.
fn legacy_defaults() -> Vec<FilenameColorRule> {
    [
        (
            concat!(
                "*.rs;*.c;*.h;*.cpp;*.hpp;*.cc;*.cs;*.java;*.kt;*.go;*.py;*.rb;*.js;*.jsx;",
                "*.ts;*.tsx;*.swift;*.sh;*.ps1;*.sql;*.html;*.css;*.scss;*.vue;*.svelte;*.lua;",
                "*.pl;*.php;*.slint;*.toml;*.yaml;*.yml;*.json;*.xml;*.cmake;Makefile;Dockerfile"
            ),
            RgbColor(70, 155, 215),
        ),
        (
            "*.exe;*.com;*.bat;*.cmd;*.msi;*.appimage;*.run",
            RgbColor(80, 170, 100),
        ),
        (
            "*.png;*.jpg;*.jpeg;*.gif;*.webp;*.bmp;*.ico;*.tif;*.tiff;*.svg;*.avif;*.heic;*.jxl",
            RgbColor(185, 120, 210),
        ),
        (
            "*.mp4;*.mkv;*.webm;*.mov;*.avi;*.mpeg;*.mpg;*.m4v;*.wmv;*.ogv",
            RgbColor(220, 135, 65),
        ),
        (
            concat!(
                "*.pdf;*.doc;*.docx;*.odt;*.rtf;*.txt;*.md;*.rst;*.xls;*.xlsx;*.ods;",
                "*.ppt;*.pptx;*.odp;*.epub;*.csv;*.tsv"
            ),
            RgbColor(60, 165, 165),
        ),
    ]
    .into_iter()
    .map(|(expression, color)| FilenameColorRule {
        expression: expression.into(),
        style: FilenameStyle {
            foreground: Some(color),
            background: None,
        },
        ..Default::default()
    })
    .collect()
}

fn reload(rules: Vec<FilenameColorRule>) -> Vec<FilenameColorRule> {
    let encoded = encode_preferences(UserPreferences {
        filename_colors: rules.into(),
        // Force a modern stored document with an explicit rule list, even when
        // those rules happen to equal the current build's default palette.
        panel_split_basis_points: 4500,
        panel_filter_history: vec![midnight_core::PanelFilter {
            enabled: true,
            expression: "*.txt".into(),
            ..Default::default()
        }],
        ..UserPreferences::default()
    })
    .unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert!(stored["filename_colors"].is_array());
    assert_eq!(stored["version"], 30, "exercise the affected saved schema");
    decode_preferences(&encoded)
        .unwrap()
        .filename_colors
        .to_vec()
}

const SKY_BLUE: RgbColor = RgbColor(35, 170, 242);
const CYAN: RgbColor = RgbColor(6, 164, 199);
const BLUE: RgbColor = RgbColor(59, 130, 246);
const WORD_DOCUMENTS: &str = "*.doc;*.docx;*.odt;*.rtf;*.epub";
const PLAIN_TEXT: &str = "*.txt;*.md;*.markdown;*.rst;*.log;*.text;README;README.*";

// The twelve-category palette as shipped before the interim cyan: blue source code and
// light cool neutral documents and text, with today's masks and other colors.
fn twelve_category_defaults() -> Vec<FilenameColorRule> {
    let mut rules = midnight_core::default_filename_colors();
    for (index, color) in [
        (0, BLUE),
        (3, RgbColor(196, 207, 224)),
        (10, RgbColor(196, 207, 224)),
    ] {
        rules[index].style.foreground = Some(color);
    }
    rules
}

// The interim palette, in which source code, Word documents and plain text/Markdown
// shared cyan, with today's masks and other colors.
fn interim_defaults() -> Vec<FilenameColorRule> {
    let mut rules = midnight_core::default_filename_colors();
    for index in [0, 3, 10] {
        rules[index].style.foreground = Some(CYAN);
    }
    rules
}

// The palette shipped until 23 November 2026: cyan source code and blue documents and
// text, with today's masks and other colors.
fn cyan_source_defaults() -> Vec<FilenameColorRule> {
    let mut rules = midnight_core::default_filename_colors();
    rules[0].style.foreground = Some(CYAN);
    rules
}

#[test]
fn defaults_color_source_sky_blue_and_documents_and_text_blue() {
    let current = midnight_core::default_filename_colors();
    let colors: Vec<_> = current
        .iter()
        .map(|rule| (rule.expression.as_str(), rule.style.foreground))
        .collect();
    assert_eq!(colors[3], (WORD_DOCUMENTS, Some(BLUE)));
    assert_eq!(colors[10], (PLAIN_TEXT, Some(BLUE)));
    assert!(colors[0].0.starts_with("*.rs;*.c;*.h;"), "{}", colors[0].0);
    assert_eq!(colors[0].1, Some(SKY_BLUE));
}

#[test]
fn unchanged_shipped_twelve_category_palettes_move_to_the_current_defaults() {
    let current = midnight_core::default_filename_colors();
    for (name, old) in [
        ("blue source, neutral documents", twelve_category_defaults()),
        ("interim cyan", interim_defaults()),
        ("cyan source, blue documents", cyan_source_defaults()),
    ] {
        let migrated = reload(old);
        assert_eq!(migrated, current, "{name}");
        assert_eq!(migrated[0].style.foreground, Some(SKY_BLUE), "{name}");
        assert_eq!(reload(migrated), current, "{name}: migration is stable");
    }
}

#[test]
fn interim_cyan_documents_and_text_turn_blue_in_a_customized_list() {
    let old = interim_defaults();
    for variation in 0..8 {
        let mut custom = old.clone();
        // The rows whose saved interim cyan turns blue.
        let blue = match variation {
            0 => {
                custom[7].style.foreground = Some(RgbColor(12, 34, 56));
                vec![3, 10]
            }
            1 => {
                custom[0].style.foreground = Some(RgbColor(1, 2, 3));
                vec![3, 10]
            }
            2 => {
                custom[3].enabled = false;
                vec![3, 10]
            }
            3 => {
                custom.swap(0, 3);
                vec![0, 10]
            }
            4 => {
                custom.pop();
                vec![3, 10]
            }
            5 => {
                custom.push(rule("*.personal", RgbColor(1, 2, 3)));
                vec![3, 10]
            }
            6 => {
                custom.remove(3);
                vec![9]
            }
            7 => {
                custom.resize(
                    midnight_core::MAX_FILENAME_COLOR_RULES,
                    rule("*.personal", RgbColor(1, 2, 3)),
                );
                vec![3, 10]
            }
            _ => unreachable!(),
        };
        let mut expected = custom.clone();
        for index in blue {
            expected[index].style.foreground = Some(BLUE);
        }
        let migrated = reload(custom);
        assert_eq!(migrated, expected, "customization {variation}");
        assert!(
            migrated
                .iter()
                .filter(|rule| rule.expression.starts_with("*.rs;"))
                .all(|rule| rule.style.foreground != Some(BLUE)),
            "customization {variation}: source code keeps its saved color"
        );
        assert_eq!(
            reload(migrated.clone()),
            migrated,
            "customization {variation}"
        );
    }
}

#[test]
fn interim_doc_text_upgrade_preserves_custom_rule_fields() {
    for expression in [WORD_DOCUMENTS, PLAIN_TEXT] {
        // The exact interim signature alone turns blue.
        assert_eq!(
            reload(vec![rule(expression, CYAN)]),
            [rule(expression, BLUE)]
        );
        for variation in 0..9 {
            let mut custom = rule(expression, CYAN);
            match variation {
                0 => custom.style.foreground = Some(RgbColor(6, 164, 200)),
                1 => custom.style.foreground = None,
                2 => custom.style.background = Some(RgbColor(1, 2, 3)),
                3 => custom.expression.push_str(";*.personal"),
                4 => custom.case_sensitive = true,
                5 => custom.target = FilenameColorTarget::All,
                6 => custom.target = FilenameColorTarget::Folders,
                7 => {
                    custom.mode = FilterMode::Regex;
                    custom.expression = r".*\.(docx|md)$".into();
                }
                8 => custom.expression = expression.to_ascii_uppercase(),
                _ => unreachable!(),
            }
            let before = vec![custom];
            assert_eq!(
                reload(before.clone()),
                before,
                "customization {variation} of {expression}"
            );
        }
    }
    // Source code keeps whatever color it was saved with, the interim cyan included.
    let source = midnight_core::default_filename_colors()[0]
        .expression
        .clone();
    for color in [CYAN, BLUE, RgbColor(1, 2, 3)] {
        let saved = vec![rule(&source, color)];
        assert_eq!(reload(saved.clone()), saved);
    }
}

#[test]
fn saved_customizations_of_the_twelve_category_palette_keep_their_colors() {
    let old = twelve_category_defaults();
    for variation in 0..8 {
        let mut custom = old.clone();
        match variation {
            0 => custom[7].style.foreground = Some(RgbColor(12, 34, 56)),
            1 => custom[0].style.foreground = Some(RgbColor(59, 130, 247)),
            2 => custom[3].enabled = false,
            3 => custom[10].expression.push_str(";*.mdx"),
            4 => custom[10].style.background = Some(RgbColor(1, 2, 3)),
            5 => custom.swap(0, 1),
            6 => {
                custom.pop();
            }
            7 => custom.push(rule("*.personal", RgbColor(1, 2, 3))),
            _ => unreachable!(),
        }
        assert_eq!(reload(custom.clone()), custom, "customization {variation}");
    }
}

#[test]
fn saved_customizations_of_the_cyan_source_palette_keep_their_colors() {
    let old = cyan_source_defaults();
    for variation in 0..9 {
        let mut custom = old.clone();
        match variation {
            0 => custom[7].style.foreground = Some(RgbColor(12, 34, 56)),
            1 => custom[0].style.foreground = Some(RgbColor(6, 164, 200)),
            2 => custom[0].enabled = false,
            3 => custom[0].expression.push_str(";*.mdx"),
            4 => custom[0].style.background = Some(RgbColor(1, 2, 3)),
            5 => custom[3].style.foreground = Some(SKY_BLUE),
            6 => custom.swap(0, 1),
            7 => {
                custom.pop();
            }
            8 => custom.push(rule("*.personal", RgbColor(1, 2, 3))),
            _ => unreachable!(),
        }
        let migrated = reload(custom.clone());
        assert_eq!(migrated, custom, "customization {variation}");
        assert_eq!(
            reload(migrated),
            custom,
            "customization {variation}: stable"
        );
    }
}

#[test]
fn exact_historical_default_palette_migrates_to_current_categories() {
    let migrated = reload(legacy_defaults());
    assert_eq!(migrated.len(), 12);
    assert_eq!(migrated, midnight_core::default_filename_colors());
    assert_eq!(reload(migrated.clone()), migrated, "migration is stable");
}

#[test]
fn any_customization_preserves_the_entire_ordered_palette() {
    let old = legacy_defaults();
    for variation in 0..11 {
        let mut custom = old.clone();
        match variation {
            0 => custom[0].enabled = false,
            1 => custom[0].mode = FilterMode::Regex,
            2 => custom[0].expression.push_str(";*.custom"),
            3 => custom[0].case_sensitive = true,
            4 => custom[0].target = FilenameColorTarget::All,
            5 => custom[0].style.foreground = Some(RgbColor(71, 155, 215)),
            6 => custom[0].style.background = Some(RgbColor(1, 2, 3)),
            7 => custom.swap(0, 1),
            8 => {
                custom.pop();
            }
            9 => custom.push(rule("*.personal", RgbColor(1, 2, 3))),
            10 => custom.clear(),
            _ => unreachable!(),
        }
        // Supply valid Regex syntax; it is still a one-rule customization.
        if variation == 1 {
            custom[0].expression = "source.*".into();
        }
        assert_eq!(reload(custom.clone()), custom, "customization {variation}");
    }
}

fn slate_document_defaults() -> Vec<FilenameColorRule> {
    let mut rules = bright_document_defaults();
    for index in [2, 9] {
        rules[index].style.foreground = Some(RgbColor(100, 116, 139));
    }
    rules
}

#[test]
fn untouched_slate_document_presets_keep_the_existing_whole_palette_migration() {
    let old = slate_document_defaults();
    let current = midnight_core::default_filename_colors();
    assert_eq!(reload(old.clone()), current);
    let mut amber = old;
    amber[7].style.foreground = Some(RgbColor(217, 119, 6));
    assert_eq!(
        reload(amber),
        current,
        "retain the earlier archive migration"
    );
}

fn bright_document_defaults() -> Vec<FilenameColorRule> {
    let mut rules = twelve_category_defaults();
    rules.remove(2);
    rules[2].expression = "*.pdf;*.doc;*.docx;*.odt;*.rtf;*.epub".into();
    rules
}

fn assert_local_pdf_split(
    before: &[FilenameColorRule],
    after: &[FilenameColorRule],
    index: usize,
    brightened_text_indices: &[usize],
) {
    assert_eq!(after.len(), before.len() + 1);
    let mut expected = before.to_vec();
    // Callers identify the fixture's eligible TEXT rows explicitly. Keep every
    // other field and row equal instead of reproducing the migration predicate.
    for &text_index in brightened_text_indices {
        expected[text_index].style.foreground = Some(RgbColor(196, 207, 224));
    }
    let mut remaining_documents = before[index].clone();
    remaining_documents.expression = "*.doc;*.docx;*.odt;*.rtf;*.epub".into();
    remaining_documents.style.foreground = Some(RgbColor(196, 207, 224));
    expected[index] = remaining_documents;
    expected.insert(
        index,
        FilenameColorRule {
            enabled: before[index].enabled,
            expression: "*.pdf".into(),
            style: FilenameStyle {
                foreground: Some(RgbColor(239, 90, 90)),
                background: None,
            },
            ..Default::default()
        },
    );
    assert_eq!(after, expected, "only the specified palette fields change");
    assert_eq!(
        reload(after.to_vec()).as_slice(),
        after,
        "migration is idempotent"
    );
}

#[test]
fn distinct_pdf_migrates_the_unchanged_bright_palette() {
    let current = midnight_core::default_filename_colors();
    assert_eq!(reload(bright_document_defaults()), current);
    assert_eq!(reload(current.clone()), current);
}

#[test]
fn pdf_split_survives_unrelated_archive_customization_and_restart() {
    for mut old in [slate_document_defaults(), bright_document_defaults()] {
        old[7].style.foreground = Some(RgbColor(12, 34, 56));
        let migrated = reload(old.clone());
        assert_local_pdf_split(&old, &migrated, 2, &[9]);
        let compiled = CompiledFilenameColors::new(&migrated).unwrap();
        assert_eq!(
            compiled.style("REPORT.PDF".as_ref(), EntryKind::File),
            Some(FilenameStyle {
                foreground: Some(RgbColor(239, 90, 90)),
                background: None,
            })
        );
        assert_eq!(
            compiled.style("letter.docx".as_ref(), EntryKind::File),
            Some(FilenameStyle {
                foreground: Some(RgbColor(196, 207, 224)),
                background: None,
            })
        );
        assert!(
            compiled
                .style("folder.pdf".as_ref(), EntryKind::Directory)
                .is_none()
        );
    }
}

#[test]
fn pdf_split_preserves_unrelated_flags_backgrounds_and_reordered_rules() {
    let mut old = slate_document_defaults();
    old[7].style.background = Some(RgbColor(1, 2, 3));
    old[7].enabled = false;
    old[9].case_sensitive = true;
    old[9].target = FilenameColorTarget::All;
    old.swap(2, 9);
    let mut personal = rule("^personal", RgbColor(4, 5, 6));
    personal.mode = FilterMode::Regex;
    personal.style.background = Some(RgbColor(7, 8, 9));
    old.insert(0, personal);
    assert_local_pdf_split(&old, &reload(old.clone()), 10, &[]);
}

#[test]
fn pdf_split_preserves_disabled_combined_category() {
    let mut old = slate_document_defaults();
    old[2].enabled = false;
    let migrated = reload(old.clone());
    assert_local_pdf_split(&old, &migrated, 2, &[9]);
    assert!(!migrated[2].enabled);
    assert!(!migrated[3].enabled);
    assert!(
        CompiledFilenameColors::new(&migrated)
            .unwrap()
            .style("report.pdf".as_ref(), EntryKind::File)
            .is_none()
    );
}

#[test]
fn pdf_split_preserves_explicit_combined_rule_customizations() {
    for variation in 0..8 {
        let mut custom = slate_document_defaults();
        match variation {
            0 => custom[2].style.foreground = Some(RgbColor(101, 116, 139)),
            1 => custom[2].style.foreground = None,
            2 => custom[2].style.background = Some(RgbColor(1, 2, 3)),
            3 => custom[2].expression.push_str(";*.personal"),
            4 => custom[2].case_sensitive = true,
            5 => custom[2].target = FilenameColorTarget::All,
            6 => custom[2].target = FilenameColorTarget::Folders,
            7 => {
                custom[2].mode = FilterMode::Regex;
                custom[2].expression = r".*\.(pdf|docx)$".into();
            }
            _ => unreachable!(),
        }
        let mut expected = custom.clone();
        expected[9].style.foreground = Some(RgbColor(196, 207, 224));
        assert_eq!(reload(custom), expected, "customization {variation}");
    }
}

#[test]
fn standalone_gray_or_custom_pdf_rules_are_never_treated_as_historical_defaults() {
    for color in [
        RgbColor(100, 116, 139),
        RgbColor(196, 207, 224),
        RgbColor(12, 34, 56),
    ] {
        let custom = vec![rule("*.pdf", color)];
        assert_eq!(reload(custom.clone()), custom);
    }
    let mut current = midnight_core::default_filename_colors();
    current[2].style.foreground = Some(RgbColor(12, 34, 56));
    assert_eq!(reload(current.clone()), current);
}

#[test]
fn pdf_split_keeps_earlier_explicit_pdf_precedence() {
    let mut old = slate_document_defaults();
    old.insert(0, rule("*.pdf", RgbColor(12, 34, 56)));
    let migrated = reload(old.clone());
    assert_local_pdf_split(&old, &migrated, 3, &[10]);
    assert_eq!(
        CompiledFilenameColors::new(&migrated)
            .unwrap()
            .style("report.pdf".as_ref(), EntryKind::File)
            .unwrap()
            .foreground,
        Some(RgbColor(12, 34, 56))
    );
}

#[test]
fn pdf_split_keeps_later_explicit_pdf_rule_shadowed_and_unchanged() {
    let mut old = slate_document_defaults();
    let custom = rule("*.pdf", RgbColor(12, 34, 56));
    old.push(custom.clone());
    let migrated = reload(old.clone());
    assert_local_pdf_split(&old, &migrated, 2, &[9]);
    assert_eq!(migrated.last(), Some(&custom));
    assert_eq!(
        CompiledFilenameColors::new(&migrated)
            .unwrap()
            .style("report.pdf".as_ref(), EntryKind::File)
            .unwrap()
            .foreground,
        Some(RgbColor(239, 90, 90))
    );
}

#[test]
fn pdf_split_respects_full_capacity_without_losing_or_rejecting_preferences() {
    let mut old = slate_document_defaults();
    old.resize(32, rule("*.personal", RgbColor(1, 2, 3)));
    let mut expected = old.clone();
    expected[9].style.foreground = Some(RgbColor(196, 207, 224));
    assert_eq!(
        reload(old.clone()),
        expected,
        "full lists keep the combined rule while eligible TEXT brightens in place"
    );
    old.pop();
    let migrated = reload(old.clone());
    assert_eq!(migrated.len(), 32);
    assert_local_pdf_split(&old, &migrated, 2, &[9]);
}

#[test]
fn pdf_split_at_capacity_with_duplicate_historical_categories_is_stable() {
    let mut old = slate_document_defaults();
    old.push(old[2].clone());
    old.resize(31, rule("*.personal", RgbColor(1, 2, 3)));
    let migrated = reload(old.clone());
    assert_local_pdf_split(&old, &migrated, 2, &[9]);
    assert_eq!(migrated.len(), 32);
}

#[test]
fn pdf_split_preserves_earlier_catch_all_style_and_unrelated_preferences() {
    let mut old = slate_document_defaults();
    let earlier = rule("*", RgbColor(1, 2, 3));
    old.insert(0, earlier.clone());
    let preferences = UserPreferences {
        filename_colors: old.clone().into(),
        panel_split_basis_points: 4500,
        ..Default::default()
    };
    let bytes = encode_preferences(preferences.clone()).unwrap();
    let decoded = decode_preferences(&bytes).unwrap();
    assert_local_pdf_split(&old, &decoded.filename_colors, 3, &[10]);
    assert_eq!(
        CompiledFilenameColors::new(&decoded.filename_colors)
            .unwrap()
            .style("report.pdf".as_ref(), EntryKind::File),
        Some(earlier.style)
    );
    let expected = UserPreferences {
        filename_colors: decoded.filename_colors.clone(),
        ..preferences
    };
    assert_eq!(decoded, expected);
}

// Reconstruct the reported saved split-category colors with the exact eligible
// historical fields. These are fixtures, not a read of live preferences.
fn saved_split_doc_text_rules() -> Vec<FilenameColorRule> {
    vec![
        rule("*.pdf", RgbColor(239, 90, 90)),
        rule("*.doc;*.docx;*.odt;*.rtf;*.epub", RgbColor(100, 116, 139)),
        rule(
            "*.txt;*.md;*.markdown;*.rst;*.log;*.text;README;README.*",
            RgbColor(100, 116, 139),
        ),
        rule(
            "*.zip;*.7z;*.rar;*.tar;*.gz;*.bz2;*.xz;*.zst;*.tgz;*.tbz2;*.txz;*.cab;*.iso",
            RgbColor(217, 207, 6),
        ),
    ]
}

#[test]
fn legacy_split_doc_and_text_gray_upgrade_without_resetting_other_rules() {
    let mut before = saved_split_doc_text_rules();
    let mut personal = rule("^personal", RgbColor(4, 5, 6));
    personal.mode = FilterMode::Regex;
    personal.enabled = false;
    personal.target = FilenameColorTarget::All;
    personal.style.background = Some(RgbColor(7, 8, 9));
    before.push(personal);
    let preferences = UserPreferences {
        filename_colors: before.clone().into(),
        panel_split_basis_points: 4500,
        panel_filter_history: vec![midnight_core::PanelFilter {
            enabled: true,
            expression: "*.txt".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let encoded = encode_preferences(preferences.clone()).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(stored["version"], 30);
    for (index, expression) in [
        (1, "*.doc;*.docx;*.odt;*.rtf;*.epub"),
        (
            2,
            "*.txt;*.md;*.markdown;*.rst;*.log;*.text;README;README.*",
        ),
    ] {
        assert_eq!(
            stored["filename_colors"][index],
            serde_json::json!({
                "enabled": true, "mode": "masks", "expression": expression,
                "case_sensitive": false, "target": "files",
                "foreground": "#64748b", "background": null
            })
        );
    }
    let decoded = decode_preferences(&encoded).unwrap();
    let mut expected = before;
    expected[1].style.foreground = Some(RgbColor(196, 207, 224));
    expected[2].style.foreground = Some(RgbColor(196, 207, 224));
    assert_eq!(
        decoded,
        UserPreferences {
            filename_colors: expected.clone().into(),
            ..preferences
        }
    );
    assert_eq!(reload(expected.clone()), expected, "second load is stable");
    let compiled = CompiledFilenameColors::new(&decoded.filename_colors).unwrap();
    for name in [
        "letter.doc",
        "LETTER.DOCX",
        "book.odt",
        "report.rtf",
        "book.epub",
        "note.txt",
        "NOTE.TXT",
        "note.md",
        "NOTE.MD",
        "notes.markdown",
        "notes.rst",
        "build.log",
        "notes.text",
        "README",
        "README.release",
    ] {
        assert_eq!(
            compiled.style(name.as_ref(), EntryKind::File),
            Some(FilenameStyle {
                foreground: Some(RgbColor(196, 207, 224)),
                background: None,
            }),
            "{name}"
        );
    }
    for (name, color) in [
        ("REPORT.PDF", RgbColor(239, 90, 90)),
        ("backup.ZIP", RgbColor(217, 207, 6)),
    ] {
        assert_eq!(
            compiled.style(name.as_ref(), EntryKind::File),
            Some(FilenameStyle {
                foreground: Some(color),
                background: None,
            }),
            "{name}"
        );
    }
    for name in ["folder.docx", "folder.txt"] {
        assert!(
            compiled
                .style(name.as_ref(), EntryKind::Directory)
                .is_none()
        );
    }
}

#[test]
fn legacy_doc_text_upgrade_follows_pdf_split_and_is_idempotent() {
    let mut before = saved_split_doc_text_rules();
    before.remove(0);
    before[0].expression = "*.pdf;*.doc;*.docx;*.odt;*.rtf;*.epub".into();
    let after = reload(before.clone());
    assert_local_pdf_split(&before, &after, 0, &[1]);
    assert_eq!(after, reload(reload(after.clone())));
}

#[test]
fn legacy_doc_text_upgrade_preserves_custom_rule_fields() {
    for expression in [
        "*.doc;*.docx;*.odt;*.rtf;*.epub",
        "*.txt;*.md;*.markdown;*.rst;*.log;*.text;README;README.*",
    ] {
        for variation in 0..10 {
            let mut custom = rule(expression, RgbColor(100, 116, 139));
            match variation {
                0 => custom.style.foreground = Some(RgbColor(101, 116, 139)),
                1 => custom.style.foreground = None,
                2 => custom.style.background = Some(RgbColor(1, 2, 3)),
                3 => custom.expression.push_str(";*.personal"),
                4 => custom.case_sensitive = true,
                5 => custom.target = FilenameColorTarget::All,
                6 => custom.target = FilenameColorTarget::Folders,
                7 => {
                    custom.mode = FilterMode::Regex;
                    custom.expression = r".*\.(docx|txt)$".into();
                }
                8 => custom.expression.push_str("|private*"),
                9 => custom.expression = expression.to_ascii_uppercase(),
                _ => unreachable!(),
            }
            let before = vec![custom];
            assert_eq!(
                reload(before.clone()),
                before,
                "customization {variation} of {expression}"
            );
        }
    }
}

#[test]
fn legacy_doc_text_upgrade_preserves_disabled_state() {
    let mut before = saved_split_doc_text_rules();
    before[1].enabled = false;
    before[2].enabled = false;
    let fallback = rule("*", RgbColor(1, 2, 3));
    before.push(fallback.clone());
    let mut expected = before.clone();
    expected[1].style.foreground = Some(RgbColor(196, 207, 224));
    expected[2].style.foreground = Some(RgbColor(196, 207, 224));
    let after = reload(before);
    assert_eq!(after, expected);
    let compiled = CompiledFilenameColors::new(&after).unwrap();
    for name in ["LETTER.DOCX", "NOTE.MD", "NOTE.TXT"] {
        assert_eq!(
            compiled.style(name.as_ref(), EntryKind::File),
            Some(fallback.style),
            "disabled category stays disabled for {name}"
        );
    }
}

#[test]
fn legacy_doc_text_upgrade_preserves_earlier_match_priority() {
    for expression in ["*", "*.docx;*.md;*.txt"] {
        let mut before = saved_split_doc_text_rules();
        let earlier = rule(expression, RgbColor(1, 2, 3));
        before.insert(0, earlier.clone());
        before.push(rule(expression, RgbColor(4, 5, 6)));
        let mut expected = before.clone();
        expected[2].style.foreground = Some(RgbColor(196, 207, 224));
        expected[3].style.foreground = Some(RgbColor(196, 207, 224));
        let after = reload(before);
        assert_eq!(after, expected);
        let compiled = CompiledFilenameColors::new(&after).unwrap();
        for name in ["LETTER.DOCX", "NOTE.MD", "NOTE.TXT"] {
            assert_eq!(
                compiled.style(name.as_ref(), EntryKind::File),
                Some(earlier.style),
                "earlier {expression} keeps priority for {name}"
            );
        }
    }
}

#[test]
fn legacy_doc_text_upgrade_keeps_full_capacity_pdf_and_archive_choices() {
    let mut before = saved_split_doc_text_rules();
    before.resize(
        midnight_core::MAX_FILENAME_COLOR_RULES,
        rule("*.personal", RgbColor(1, 2, 3)),
    );
    let last = before.last_mut().unwrap();
    last.enabled = false;
    last.case_sensitive = true;
    last.target = FilenameColorTarget::All;
    last.style.background = Some(RgbColor(7, 8, 9));
    let mut expected = before.clone();
    expected[1].style.foreground = Some(RgbColor(196, 207, 224));
    expected[2].style.foreground = Some(RgbColor(196, 207, 224));
    let after = reload(before);
    assert_eq!(after.len(), midnight_core::MAX_FILENAME_COLOR_RULES);
    assert_eq!(after, expected, "no slot is needed for in-place promotion");
    assert_eq!(reload(after.clone()), after);
    let compiled = CompiledFilenameColors::new(&after).unwrap();
    for (name, color) in [
        ("LETTER.DOCX", RgbColor(196, 207, 224)),
        ("NOTE.MD", RgbColor(196, 207, 224)),
        ("NOTE.TXT", RgbColor(196, 207, 224)),
        ("REPORT.PDF", RgbColor(239, 90, 90)),
        ("backup.ZIP", RgbColor(217, 207, 6)),
    ] {
        assert_eq!(
            compiled.style(name.as_ref(), EntryKind::File),
            Some(FilenameStyle {
                foreground: Some(color),
                background: None,
            }),
            "{name}"
        );
    }
}

#[test]
fn legacy_doc_text_upgrade_does_not_restore_deleted_or_individual_categories() {
    for (deleted, missing_name) in [(1, "letter.docx"), (2, "note.txt")] {
        let mut before = saved_split_doc_text_rules();
        before.remove(deleted);
        let mut expected = before.clone();
        expected[1].style.foreground = Some(RgbColor(196, 207, 224));
        let after = reload(before);
        assert_eq!(after, expected);
        assert!(
            CompiledFilenameColors::new(&after)
                .unwrap()
                .style(missing_name.as_ref(), EntryKind::File)
                .is_none()
        );
    }
    assert!(reload(Vec::new()).is_empty());
    let individual = ["*.pdf", "*.docx", "*.txt", "*.md"]
        .into_iter()
        .map(|expression| rule(expression, RgbColor(100, 116, 139)))
        .collect::<Vec<_>>();
    assert_eq!(reload(individual.clone()), individual);
}
