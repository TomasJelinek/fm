use midnight_core::{PreferenceTheme, UiDensity, UserPreferences};
use midnight_platform_common::settings::{decode_preferences, encode_preferences};
use serde_json::{Value, json};
use std::io;

fn document(preferences: UserPreferences) -> Value {
    serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap()
}

#[test]
fn every_theme_roundtrips_with_an_independently_selected_opposite_appearance() {
    let custom = UserPreferences {
        dark_theme: PreferenceTheme::PlumSmoke,
        light_theme: PreferenceTheme::Iris,
        density: UiDensity::Touch,
        font_family: "Stored 文档 e\u{301}".into(),
        desktop_font_size: 19,
        touch_font_size: 27,
        ..Default::default()
    };
    for theme in PreferenceTheme::ALL {
        let mut expected = custom.clone();
        if theme.is_light() {
            expected.light_theme = theme;
        } else {
            expected.dark_theme = theme;
        }
        let saved = document(expected.clone());
        assert_eq!(
            saved["version"],
            midnight_platform_common::settings::SETTINGS_FILE_VERSION
        );
        assert!(saved.get("theme").is_none());
        assert_eq!(saved["dark_theme"], expected.dark_theme.storage_name());
        assert_eq!(saved["light_theme"], expected.light_theme.storage_name());
        assert_eq!(
            decode_preferences(&serde_json::to_vec(&saved).unwrap()).unwrap(),
            expected
        );
    }
}

#[test]
fn default_pair_preserves_the_minimum_schema_and_legacy_themes_use_the_defaults() {
    let defaults = UserPreferences::default();
    let expected = document(defaults.clone());
    assert_eq!(expected["version"], 10);
    assert_eq!(expected["theme"], "blue");
    assert!(expected.get("dark_theme").is_none());
    assert!(expected.get("light_theme").is_none());
    let mut old = expected.clone();
    for name in PreferenceTheme::ALL
        .into_iter()
        .take(26)
        .map(PreferenceTheme::storage_name)
        .chain(["dos-commander"])
    {
        old["theme"] = name.into();
        let loaded = decode_preferences(&serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(loaded, defaults);
        assert_eq!(document(loaded), expected);
    }
    for name in ["plum-smoke", "future-theme", "Nord"] {
        old["theme"] = name.into();
        assert!(decode_preferences(&serde_json::to_vec(&old).unwrap()).is_err());
    }
}

#[test]
fn either_custom_selection_stores_both_themes_and_restoring_defaults_returns_to_the_old_schema() {
    for prefs in [
        UserPreferences {
            dark_theme: PreferenceTheme::PlumSmoke,
            ..Default::default()
        },
        UserPreferences {
            light_theme: PreferenceTheme::Iris,
            ..Default::default()
        },
    ] {
        let custom = document(prefs.clone());
        assert_eq!(custom["version"], 45);
        assert!(custom.get("theme").is_none());
        assert_eq!(custom["dark_theme"], prefs.dark_theme.storage_name());
        assert_eq!(custom["light_theme"], prefs.light_theme.storage_name());
        let mut loaded = decode_preferences(&serde_json::to_vec(&custom).unwrap()).unwrap();
        assert_eq!(loaded, prefs);
        loaded.dark_theme = PreferenceTheme::DEFAULT_DARK;
        loaded.light_theme = PreferenceTheme::DEFAULT_LIGHT;
        assert_eq!(document(loaded), document(UserPreferences::default()));
    }
}

#[test]
fn separate_theme_keys_require_both_recognized_names_in_the_correct_mode() {
    let original = document(UserPreferences {
        dark_theme: PreferenceTheme::PlumSmoke,
        ..Default::default()
    });
    for (field, value) in [
        ("light_theme", json!("plum-smoke")),
        ("dark_theme", json!("letterhead")),
        ("dark_theme", Value::Null),
        ("light_theme", json!(2)),
        ("dark_theme", json!("")),
        ("light_theme", json!("solarized-light ")),
        ("dark_theme", json!("future-theme")),
        ("dark_theme", json!("Nord")),
        ("dark_theme", json!("gruvbox_dark")),
        ("theme", json!("blue")),
    ] {
        let mut invalid = original.clone();
        invalid[field] = value;
        assert_eq!(
            decode_preferences(&serde_json::to_vec(&invalid).unwrap())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
    for field in ["light_theme", "dark_theme"] {
        let mut missing = original.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(decode_preferences(&serde_json::to_vec(&missing).unwrap()).is_err());
    }
    let wrong = UserPreferences {
        dark_theme: PreferenceTheme::Iris,
        ..Default::default()
    };
    assert!(encode_preferences(wrong).is_err());
    let mut legacy = document(UserPreferences::default());
    legacy["dark_theme"] = "blue".into();
    assert!(decode_preferences(&serde_json::to_vec(&legacy).unwrap()).is_err());
}

#[test]
fn dark_and_light_are_the_defaults_and_explicit_choices_are_kept() {
    assert_eq!(PreferenceTheme::DEFAULT_DARK, PreferenceTheme::Dark);
    assert_eq!(PreferenceTheme::DEFAULT_LIGHT, PreferenceTheme::Light);
    let old = document(UserPreferences::default());
    assert!(old.get("dark_theme").is_none());
    assert!(old.get("light_theme").is_none());
    let loaded = decode_preferences(&serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(loaded.dark_theme, PreferenceTheme::Dark);
    assert_eq!(loaded.light_theme, PreferenceTheme::Light);
    let previous_defaults = UserPreferences {
        dark_theme: PreferenceTheme::PencilLead,
        light_theme: PreferenceTheme::Letterhead,
        ..Default::default()
    };
    let saved = document(previous_defaults.clone());
    assert_eq!(saved["dark_theme"], "pencil-lead");
    assert_eq!(saved["light_theme"], "letterhead");
    assert_eq!(
        decode_preferences(&serde_json::to_vec(&saved).unwrap()).unwrap(),
        previous_defaults
    );
}
