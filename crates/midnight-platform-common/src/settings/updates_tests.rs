//! Schema 40: the update check choices (Options › General › Updates).
use super::{decode_preferences, encode_preferences};
use midnight_core::{UpdatePreferences, UserPreferences, WheelSpeed};
use serde_json::{Value, json};

fn stored(preferences: &UserPreferences) -> Value {
    serde_json::from_slice(&encode_preferences(preferences.clone()).unwrap()).unwrap()
}

fn decoded(value: &Value) -> std::io::Result<UserPreferences> {
    decode_preferences(&serde_json::to_vec(value).unwrap())
}

#[test]
fn both_choices_are_on_by_default_and_need_no_new_schema() {
    let preferences = UserPreferences::default();
    assert!(preferences.updates.check_at_startup);
    assert!(preferences.updates.previews);
    let value = stored(&preferences);
    assert_eq!(value["version"], 10);
    assert!(value.get("updates").is_none());
}

#[test]
fn settings_from_before_schema_40_turn_the_startup_check_on() {
    // A schema 38 file (a wheel speed choice) has no update choices.
    let schema_38 = stored(&UserPreferences {
        wheel_speed: WheelSpeed::Fast,
        ..Default::default()
    });
    assert_eq!(schema_38["version"], 38);
    let read = decoded(&schema_38).unwrap();
    assert_eq!(read.updates, UpdatePreferences::default());
    assert!(read.updates.check_at_startup);
    // So does the minimum schema that the defaults write.
    let minimum = stored(&UserPreferences::default());
    assert_eq!(minimum["version"], 10);
    assert!(decoded(&minimum).unwrap().updates.check_at_startup);
}

#[test]
fn a_choice_that_is_off_writes_schema_40_and_round_trips() {
    for (check_at_startup, previews) in [(false, true), (true, false), (false, false)] {
        let preferences = UserPreferences {
            updates: UpdatePreferences {
                check_at_startup,
                previews,
            },
            ..Default::default()
        };
        let value = stored(&preferences);
        assert_eq!(value["version"], 40);
        assert_eq!(
            value["updates"],
            json!({"check_at_startup": check_at_startup, "previews": previews})
        );
        // Fields that schemas 30 to 34 require are present.
        assert_eq!(value["disk_io_concurrency"], json!({"mode": "automatic"}));
        assert_eq!(decoded(&value).unwrap(), preferences);
    }
}

#[test]
fn malformed_update_choices_are_refused() {
    let mut value = stored(&UserPreferences {
        updates: UpdatePreferences {
            check_at_startup: false,
            previews: true,
        },
        ..Default::default()
    });
    // Version 40 without its field.
    let mut missing = value.clone();
    missing.as_object_mut().unwrap().remove("updates");
    assert!(decoded(&missing).is_err());
    // Unknown fields.
    value["updates"]["channel"] = json!("nightly");
    assert!(decoded(&value).is_err());
    // An update choice in an older schema.
    let mut older = stored(&UserPreferences {
        wheel_speed: WheelSpeed::Fast,
        ..Default::default()
    });
    older["updates"] = json!({"check_at_startup": false, "previews": false});
    assert!(decoded(&older).is_err());
}
