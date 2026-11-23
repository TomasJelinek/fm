//! Schema 35: Show hidden files and the last Select/Deselect by mask choices.
use super::{decode_preferences, encode_preferences};
use midnight_core::{
    GlyphFitting, MaskSelectionPreferences, ThumbnailerAssociations, UserPreferences,
};
use serde_json::{Value, json};
use std::sync::Arc;

fn stored(preferences: &UserPreferences) -> Value {
    serde_json::from_slice(&encode_preferences(preferences.clone()).unwrap()).unwrap()
}

fn decoded(value: &Value) -> std::io::Result<UserPreferences> {
    decode_preferences(&serde_json::to_vec(value).unwrap())
}

#[test]
fn default_choices_keep_the_previous_minimum_schema_and_show_hidden_files() {
    let defaults = UserPreferences::default();
    assert!(defaults.show_hidden_files);
    assert_eq!(defaults.mask_selection, MaskSelectionPreferences::default());
    let value = stored(&defaults);
    assert_eq!(value["version"], 10);
    assert!(value.get("show_hidden_files").is_none());
    assert!(value.get("mask_selection").is_none());
    assert_eq!(decoded(&value).unwrap(), defaults);
}

#[test]
fn hidden_files_choice_writes_schema_35_with_every_field_schemas_33_and_34_require() {
    let preferences = UserPreferences {
        show_hidden_files: false,
        ..Default::default()
    };
    let value = stored(&preferences);
    assert_eq!(value["version"], 35);
    assert_eq!(value["show_hidden_files"], false);
    assert!(value.get("mask_selection").is_none());
    assert_eq!(value["file_comparators"], json!({"rules": []}));
    assert_eq!(value["disk_io_concurrency"], json!({"mode": "automatic"}));
    assert_eq!(value["previews"]["archives"], true);
    assert!(value.get("thumbnailer_associations").is_none());
    assert!(value.get("glyph_fitting").is_none());
    assert_eq!(decoded(&value).unwrap(), preferences);

    // Showing hidden files again restores the previous minimum schema.
    let shown = stored(&UserPreferences::default());
    assert_ne!(shown["version"], 35);
}

#[test]
fn mask_choice_round_trips_with_unicode_and_the_folder_option() {
    let preferences = UserPreferences {
        mask_selection: MaskSelectionPreferences {
            pattern: "文档*.txt;*.md|draft*".into(),
            folders: true,
        },
        ..Default::default()
    };
    let value = stored(&preferences);
    assert_eq!(value["version"], 35);
    assert_eq!(
        value["mask_selection"],
        json!({"pattern": "文档*.txt;*.md|draft*", "folders": true})
    );
    assert!(value.get("show_hidden_files").is_none());
    assert_eq!(decoded(&value).unwrap(), preferences);

    let both = UserPreferences {
        show_hidden_files: false,
        glyph_fitting: GlyphFitting::Subpixel,
        thumbnailer_associations: Arc::new(ThumbnailerAssociations {
            overrides: [("image.lab".into(), vec!["png".into()])].into(),
        }),
        ..preferences
    };
    let value = stored(&both);
    assert_eq!(value["version"], 35);
    assert_eq!(value["glyph_fitting"], "subpixel");
    assert_eq!(decoded(&value).unwrap(), both);
}

#[test]
fn schema_33_and_34_documents_load_unchanged_with_hidden_files_shown() {
    let thumbnails = UserPreferences {
        thumbnailer_associations: Arc::new(ThumbnailerAssociations {
            overrides: [("image.lab".into(), vec!["bmp".into()])].into(),
        }),
        ..Default::default()
    };
    let schema_34 = stored(&thumbnails);
    assert_eq!(schema_34["version"], 34);
    let loaded = decoded(&schema_34).unwrap();
    assert_eq!(loaded, thumbnails);
    assert!(loaded.show_hidden_files);
    assert_eq!(loaded.mask_selection, MaskSelectionPreferences::default());

    let mut schema_33 = schema_34.clone();
    schema_33["version"] = 33.into();
    schema_33
        .as_object_mut()
        .unwrap()
        .remove("thumbnailer_associations");
    let loaded = decoded(&schema_33).unwrap();
    assert!(loaded.show_hidden_files);
    assert_eq!(loaded.mask_selection, MaskSelectionPreferences::default());
}

#[test]
fn schema_35_fields_are_versioned_strict_and_future_versions_stay_refused() {
    let valid = stored(&UserPreferences {
        show_hidden_files: false,
        mask_selection: MaskSelectionPreferences {
            pattern: "*.log".into(),
            folders: false,
        },
        ..Default::default()
    });
    assert!(decoded(&valid).is_ok());

    // A schema 35 document needs one of its own fields.
    let mut missing = valid.clone();
    let object = missing.as_object_mut().unwrap();
    object.remove("show_hidden_files");
    object.remove("mask_selection");
    assert!(decoded(&missing).is_err());

    // Earlier schemas cannot carry the new fields.
    for version in [33, 34] {
        let mut premature = valid.clone();
        premature["version"] = version.into();
        premature["glyph_fitting"] = "subpixel".into();
        assert!(decoded(&premature).is_err(), "version {version}");
    }

    for (field, invalid) in [
        ("show_hidden_files", Value::Null),
        ("show_hidden_files", json!("false")),
        ("mask_selection", Value::Null),
        ("mask_selection", json!({"pattern": "*"})),
        (
            "mask_selection",
            json!({"pattern": "*", "folders": false, "case": true}),
        ),
        (
            "mask_selection",
            json!({"pattern": "*\u{7}", "folders": false}),
        ),
    ] {
        let mut mutated = valid.clone();
        mutated[field] = invalid.clone();
        assert!(decoded(&mutated).is_err(), "{field}: {invalid}");
    }

    let duplicate = serde_json::to_string(&valid).unwrap().replacen(
        "\"show_hidden_files\":false",
        "\"show_hidden_files\":false,\"show_hidden_files\":true",
        1,
    );
    assert!(decode_preferences(duplicate.as_bytes()).is_err());

    let mut future = valid;
    future["version"] = 41.into();
    let error = decoded(&future).unwrap_err();
    assert_eq!(error.to_string(), "unsupported preferences version");
}
