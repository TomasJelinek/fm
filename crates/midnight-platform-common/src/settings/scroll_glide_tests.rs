//! Schema 37: the scrolling glide (Options › Appearance), in percent of the default glide.
use super::{decode_preferences, decode_settings_file, encode_preferences, encode_settings_file};
use midnight_core::{DocumentLayout, ScrollGlide, UserPreferences, ViewerPreferenceChange};
use serde_json::{Value, json};

fn stored(preferences: &UserPreferences) -> Value {
    serde_json::from_slice(&encode_preferences(preferences.clone()).unwrap()).unwrap()
}

fn decoded(value: &Value) -> std::io::Result<UserPreferences> {
    decode_preferences(&serde_json::to_vec(value).unwrap())
}

fn glide(scroll_glide: ScrollGlide) -> UserPreferences {
    UserPreferences {
        scroll_glide,
        ..Default::default()
    }
}

#[test]
fn the_default_glide_keeps_the_previous_minimum_schema_and_old_files_read_as_default() {
    let value = stored(&UserPreferences::default());
    assert_eq!(value["version"], 10);
    assert!(value.get("scroll_glide").is_none());
    assert_eq!(decoded(&value).unwrap().scroll_glide, ScrollGlide::Default);
    // Files of every earlier schema have no glide and read as today's glide.
    let layout = {
        let mut preferences = UserPreferences::default();
        preferences
            .viewer
            .apply(&ViewerPreferenceChange::DocumentLayout(
                DocumentLayout::Continuous,
            ))
            .unwrap();
        preferences
    };
    let schema_36 = stored(&layout);
    assert_eq!(schema_36["version"], 36);
    assert_eq!(
        decoded(&schema_36).unwrap().scroll_glide,
        ScrollGlide::Default
    );
}

#[test]
fn every_other_glide_writes_schema_37_and_round_trips() {
    for choice in ScrollGlide::ALL {
        if choice == ScrollGlide::Default {
            continue;
        }
        let preferences = glide(choice);
        let value = stored(&preferences);
        assert_eq!(value["version"], 37, "{choice:?}");
        assert_eq!(value["scroll_glide"], choice.percent(), "{choice:?}");
        // Fields that schemas 30 to 34 require are present; later schemas' own are not.
        assert_eq!(value["file_comparators"], json!({"rules": []}));
        assert_eq!(value["disk_io_concurrency"], json!({"mode": "automatic"}));
        assert!(value.get("show_hidden_files").is_none());
        assert_eq!(decoded(&value).unwrap(), preferences, "{choice:?}");
    }
    assert_eq!(stored(&glide(ScrollGlide::Off))["scroll_glide"], 0);

    // Choosing the default again restores the previous minimum schema.
    let value = stored(&glide(ScrollGlide::Default));
    assert_eq!(value["version"], 10);
    assert!(value.get("scroll_glide").is_none());
}

#[test]
fn the_glide_survives_settings_export_and_import() {
    for choice in [ScrollGlide::Off, ScrollGlide::VeryLong] {
        let file = encode_settings_file(glide(choice)).unwrap();
        let imported = decode_settings_file(&file).unwrap();
        assert_eq!(imported.preferences.scroll_glide, choice);
    }
}

#[test]
fn schema_37_fields_are_versioned_strict_and_future_versions_stay_refused() {
    let valid = stored(&glide(ScrollGlide::Long));
    assert!(decoded(&valid).is_ok());

    // A schema 37 document needs its own field.
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("scroll_glide");
    assert!(decoded(&missing).is_err());

    // Earlier schemas cannot carry the glide.
    for version in [21, 34, 36] {
        let mut premature = valid.clone();
        premature["version"] = version.into();
        premature["glyph_fitting"] = "snapped".into();
        premature["show_hidden_files"] = false.into();
        assert!(decoded(&premature).is_err(), "version {version}");
    }

    for invalid in [
        Value::Null,
        json!(-1),
        json!(1.5),
        json!(150),
        json!(401),
        json!(70000),
        json!("long"),
    ] {
        let mut mutated = valid.clone();
        mutated["scroll_glide"] = invalid.clone();
        assert!(decoded(&mutated).is_err(), "{invalid}");
    }

    let mut future = valid;
    future["version"] = 40.into();
    let error = decoded(&future).unwrap_err();
    assert_eq!(error.to_string(), "unsupported preferences version");
}
