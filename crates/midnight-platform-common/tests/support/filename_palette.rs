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
        ..UserPreferences::default()
    })
    .unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert!(stored["filename_colors"].is_array());
    decode_preferences(&encoded)
        .unwrap()
        .filename_colors
        .to_vec()
}

#[test]
fn exact_historical_default_palette_migrates_to_eleven_requested_categories() {
    let migrated = reload(legacy_defaults());
    assert_eq!(migrated.len(), 11);
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
    let mut rules = midnight_core::default_filename_colors();
    for index in [2, 9] {
        rules[index].style.foreground = Some(RgbColor(100, 116, 139));
    }
    rules
}

#[test]
fn untouched_slate_document_presets_migrate_without_replacing_custom_colors() {
    let old = slate_document_defaults();
    let current = midnight_core::default_filename_colors();
    assert_ne!(
        old, current,
        "documents must have a brighter starting color"
    );
    assert_eq!(reload(old.clone()), current);
    let mut amber = old.clone();
    amber[7].style.foreground = Some(RgbColor(217, 119, 6));
    assert_eq!(
        reload(amber),
        current,
        "retain the earlier archive migration"
    );
    for variation in 0..5 {
        let mut custom = old.clone();
        match variation {
            0 => custom[2].style.foreground = Some(RgbColor(101, 116, 139)),
            1 => custom[9].style.background = Some(RgbColor(1, 2, 3)),
            2 => custom[9].enabled = false,
            3 => custom.swap(2, 9),
            4 => custom[0].expression.push_str(";*.personal"),
            _ => unreachable!(),
        }
        assert_eq!(reload(custom.clone()), custom, "customization {variation}");
    }
}
