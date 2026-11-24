//! Schema 43: Options › General › File operations › Preserve times and attributes.
use super::{decode_preferences, encode_preferences};
use midnight_core::{FindPreferences, UpdatePreferences, UserPreferences};
use serde_json::{Value, json};

fn stored(preferences: &UserPreferences) -> Value {
    serde_json::from_slice(&encode_preferences(preferences.clone()).unwrap()).unwrap()
}

fn decoded(value: &Value) -> std::io::Result<UserPreferences> {
    decode_preferences(&serde_json::to_vec(value).unwrap())
}

#[test]
fn preserving_is_off_by_default_and_needs_no_new_schema() {
    let preferences = UserPreferences::default();
    assert!(!preferences.copy_preserve_metadata);
    let value = stored(&preferences);
    assert_eq!(value["version"], 10);
    assert!(value.get("copy_preserve_metadata").is_none());
}

#[test]
fn the_choice_on_writes_schema_43_and_round_trips() {
    let preferences = UserPreferences {
        copy_preserve_metadata: true,
        ..Default::default()
    };
    let value = stored(&preferences);
    assert_eq!(value["version"], 43);
    assert_eq!(value["copy_preserve_metadata"], json!(true));
    // Fields that schemas 30 to 34 require are present.
    assert_eq!(value["disk_io_concurrency"], json!({"mode": "automatic"}));
    assert_eq!(decoded(&value).unwrap(), preferences);
}

#[test]
fn earlier_schemas_migrate_to_off() {
    // A schema 40 file (an update choice) has no saved choice: the former per-dialog
    // "Preserve exact modified time" was never saved, so nothing else carries over.
    let schema_40 = stored(&UserPreferences {
        updates: UpdatePreferences {
            check_at_startup: false,
            previews: true,
        },
        ..Default::default()
    });
    assert_eq!(schema_40["version"], 40);
    assert!(!decoded(&schema_40).unwrap().copy_preserve_metadata);
    let minimum = stored(&UserPreferences::default());
    assert!(!decoded(&minimum).unwrap().copy_preserve_metadata);
}

#[test]
fn malformed_choices_are_refused() {
    let value = stored(&UserPreferences {
        copy_preserve_metadata: true,
        ..Default::default()
    });
    // Version 43 without its field.
    let mut missing = value.clone();
    missing
        .as_object_mut()
        .unwrap()
        .remove("copy_preserve_metadata");
    assert!(decoded(&missing).is_err());
    // Not a boolean.
    let mut text = value.clone();
    text["copy_preserve_metadata"] = json!("yes");
    assert!(decoded(&text).is_err());
    // The choice in an older schema.
    let mut older = stored(&UserPreferences {
        updates: UpdatePreferences {
            check_at_startup: false,
            previews: false,
        },
        ..Default::default()
    });
    older["copy_preserve_metadata"] = json!(true);
    assert!(decoded(&older).is_err());
    // A future schema.
    let mut future = value;
    future["version"] = (super::CURRENT_PREFERENCES_VERSION + 1).into();
    assert!(decoded(&future).is_err());
}

#[test]
fn schema_43_keeps_find_and_needs_neither_it_nor_nested_archives() {
    let both = UserPreferences {
        copy_preserve_metadata: true,
        find: FindPreferences {
            mask_history: vec!["*.rs".into()],
            ..Default::default()
        },
        ..Default::default()
    };
    let value = stored(&both);
    assert_eq!(value["version"], 43);
    assert!(value.get("find").is_some());
    assert_eq!(decoded(&value).unwrap(), both);
    // Without the Find field a schema 43 file still reads (Find's rule is its own 41).
    let mut without_find = value;
    without_find.as_object_mut().unwrap().remove("find");
    let read = decoded(&without_find).unwrap();
    assert!(read.copy_preserve_metadata);
    assert_eq!(read.find, FindPreferences::default());
}
