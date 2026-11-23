//! Schema 36: the PDF layout chosen last (Paged or Continuous), application-wide.
use super::{decode_preferences, decode_settings_file, encode_preferences, encode_settings_file};
use midnight_core::{DocumentLayout, UserPreferences, ViewerPreferenceChange, ViewerPreferences};
use serde_json::{Value, json};

fn stored(preferences: &UserPreferences) -> Value {
    serde_json::from_slice(&encode_preferences(preferences.clone()).unwrap()).unwrap()
}

fn decoded(value: &Value) -> std::io::Result<UserPreferences> {
    decode_preferences(&serde_json::to_vec(value).unwrap())
}

fn continuous() -> UserPreferences {
    let mut viewer = ViewerPreferences::default();
    viewer
        .apply(&ViewerPreferenceChange::DocumentLayout(
            DocumentLayout::Continuous,
        ))
        .unwrap();
    UserPreferences {
        viewer,
        ..Default::default()
    }
}

#[test]
fn the_paged_default_keeps_the_previous_minimum_schema() {
    let value = stored(&UserPreferences::default());
    assert_eq!(value["version"], 10);
    assert!(value.get("viewer").is_none());
    assert_eq!(
        decoded(&value).unwrap().viewer.document_layout,
        DocumentLayout::Paged
    );
}

#[test]
fn continuous_writes_schema_36_with_every_field_earlier_schemas_require() {
    let preferences = continuous();
    let value = stored(&preferences);
    assert_eq!(value["version"], 36);
    assert_eq!(value["viewer"]["document_layout"], "continuous");
    // Fields that schemas 30 to 35 require are present; schema 35's own choices are not.
    assert_eq!(value["file_comparators"], json!({"rules": []}));
    assert_eq!(value["disk_io_concurrency"], json!({"mode": "automatic"}));
    assert_eq!(value["previews"]["archives"], true);
    assert!(value.get("show_hidden_files").is_none());
    assert!(value.get("mask_selection").is_none());
    let loaded = decoded(&value).unwrap();
    assert_eq!(loaded, preferences);
    assert_eq!(loaded.viewer.document_layout, DocumentLayout::Continuous);

    // Choosing Paged again restores the previous minimum schema and default viewer section.
    let mut paged = loaded;
    paged
        .viewer
        .apply(&ViewerPreferenceChange::DocumentLayout(
            DocumentLayout::Paged,
        ))
        .unwrap();
    let value = stored(&paged);
    assert_ne!(value["version"], 36);
    assert!(value.get("viewer").is_none());

    // Hidden files and the layout together still write schema 36 with both choices.
    let both = UserPreferences {
        show_hidden_files: false,
        ..continuous()
    };
    let value = stored(&both);
    assert_eq!(value["version"], 36);
    assert_eq!(value["show_hidden_files"], false);
    assert_eq!(decoded(&value).unwrap(), both);
}

#[test]
fn the_layout_survives_settings_export_and_import() {
    let preferences = continuous();
    let file = encode_settings_file(preferences.clone()).unwrap();
    let imported = decode_settings_file(&file).unwrap();
    assert_eq!(
        imported.preferences.viewer.document_layout,
        DocumentLayout::Continuous
    );
    assert_eq!(imported.preferences.viewer, preferences.viewer);
}

#[test]
fn schema_36_fields_are_versioned_strict_and_future_versions_stay_refused() {
    let valid = stored(&continuous());
    assert!(decoded(&valid).is_ok());

    // A schema 36 document needs its own field.
    let mut missing = valid.clone();
    missing["viewer"]
        .as_object_mut()
        .unwrap()
        .remove("document_layout");
    assert!(decoded(&missing).is_err());

    // Earlier schemas cannot carry the layout.
    for version in [21, 33, 35] {
        let mut premature = valid.clone();
        premature["version"] = version.into();
        premature["show_hidden_files"] = false.into();
        assert!(decoded(&premature).is_err(), "version {version}");
    }

    for invalid in [
        Value::Null,
        json!("Continuous"),
        json!("scroll"),
        json!(1),
        json!({"mode": "continuous"}),
    ] {
        let mut mutated = valid.clone();
        mutated["viewer"]["document_layout"] = invalid.clone();
        assert!(decoded(&mutated).is_err(), "{invalid}");
    }

    let mut future = valid;
    future["version"] = 39.into();
    let error = decoded(&future).unwrap_err();
    assert_eq!(error.to_string(), "unsupported preferences version");
}
