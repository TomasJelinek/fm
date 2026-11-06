use midnight_core::{UiDensity, UserPreferences};
use midnight_platform_common::settings::{decode_preferences, encode_preferences};
use std::{fs, io, path::Path};

const THEME_NAMES: [&str; 8] = [
    "blue",
    "dark",
    "light",
    "dracula",
    "nord",
    "gruvbox-dark",
    "catppuccin-mocha",
    "solarized-light",
];

#[test]
fn all_eight_theme_names_roundtrip_without_changing_the_schema_or_other_preferences() {
    for name in THEME_NAMES {
        let preferences = UserPreferences {
            density: UiDensity::Touch,
            font_family: "Stored 文档 e\u{301}".into(),
            desktop_font_size: 19,
            touch_font_size: 27,
            quick_search_filter: true,
            ..Default::default()
        };
        let mut input: serde_json::Value =
            serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
        input["theme"] = name.into();
        let loaded = decode_preferences(&serde_json::to_vec(&input).unwrap())
            .unwrap_or_else(|error| panic!("theme {name}: {error}"));
        let output: serde_json::Value =
            serde_json::from_slice(&encode_preferences(loaded).unwrap()).unwrap();
        assert_eq!(output, input, "theme {name}");
        assert_eq!(output["version"], 8);
    }
}

#[test]
fn all_previous_schema_versions_keep_the_original_theme_names() {
    for name in &THEME_NAMES[..3] {
        for version in 1..=7 {
            let mut input: serde_json::Value =
                serde_json::from_slice(&encode_preferences(UserPreferences::default()).unwrap())
                    .unwrap();
            input["version"] = version.into();
            input["theme"] = (*name).into();
            let fields = [
                (2, "font_family"),
                (2, "desktop_font_size"),
                (2, "touch_font_size"),
                (3, "bookmarks"),
                (4, "quick_search_filter"),
                (5, "size_display"),
                (5, "remember_command_history"),
                (5, "command_history"),
                (6, "tabs"),
                (6, "panel_view_modes"),
                (7, "shortcuts"),
                (8, "panel_filters"),
            ];
            for (introduced, field) in fields {
                if version < introduced {
                    input.as_object_mut().unwrap().remove(field);
                }
            }
            let loaded = decode_preferences(&serde_json::to_vec(&input).unwrap()).unwrap();
            let output: serde_json::Value =
                serde_json::from_slice(&encode_preferences(loaded).unwrap()).unwrap();
            assert_eq!(output["theme"], *name, "version {version}");
            assert_eq!(output["version"], 8);
        }
    }
}

#[test]
fn unknown_theme_names_are_rejected_without_changing_the_stored_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    fs::create_dir_all(&root).unwrap();
    let directory = tempfile::Builder::new()
        .prefix("theme-preferences-")
        .tempdir_in(root)
        .unwrap();
    let path = directory.path().join("preferences.json");
    for name in [
        "future-theme",
        "Nord",
        "gruvbox_dark",
        "",
        "solarized-light ",
    ] {
        let mut input: serde_json::Value =
            serde_json::from_slice(&encode_preferences(UserPreferences::default()).unwrap())
                .unwrap();
        input["theme"] = name.into();
        let bytes = serde_json::to_vec(&input).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert_eq!(
            decode_preferences(&fs::read(&path).unwrap())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData,
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
