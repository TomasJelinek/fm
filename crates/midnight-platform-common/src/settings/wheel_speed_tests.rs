//! Schema 38: the mouse-wheel speed (Options › Appearance), in percent of the system's notch.
use super::{decode_preferences, decode_settings_file, encode_preferences, encode_settings_file};
use midnight_core::{ScrollGlide, UserPreferences, WheelSpeed};
use serde_json::{Value, json};

fn stored(preferences: &UserPreferences) -> Value {
    serde_json::from_slice(&encode_preferences(preferences.clone()).unwrap()).unwrap()
}

fn decoded(value: &Value) -> std::io::Result<UserPreferences> {
    decode_preferences(&serde_json::to_vec(value).unwrap())
}

fn speed(wheel_speed: WheelSpeed) -> UserPreferences {
    UserPreferences {
        wheel_speed,
        ..Default::default()
    }
}

#[test]
fn the_medium_speed_keeps_the_previous_minimum_schema_and_old_files_read_as_medium() {
    let value = stored(&UserPreferences::default());
    assert_eq!(value["version"], 10);
    assert!(value.get("wheel_speed").is_none());
    assert_eq!(decoded(&value).unwrap().wheel_speed, WheelSpeed::Medium);
    // A schema 37 file (a glide choice) has no speed and reads as Medium.
    let glide = UserPreferences {
        scroll_glide: ScrollGlide::Long,
        ..Default::default()
    };
    let schema_37 = stored(&glide);
    assert_eq!(schema_37["version"], 37);
    assert!(schema_37.get("wheel_speed").is_none());
    assert_eq!(decoded(&schema_37).unwrap().wheel_speed, WheelSpeed::Medium);
}

#[test]
fn every_other_speed_writes_schema_38_and_round_trips() {
    for choice in WheelSpeed::ALL {
        if choice == WheelSpeed::Medium {
            continue;
        }
        let preferences = speed(choice);
        let value = stored(&preferences);
        assert_eq!(value["version"], 38, "{choice:?}");
        assert_eq!(value["wheel_speed"], choice.percent(), "{choice:?}");
        // Fields that schemas 30 to 34 require are present; later schemas' own are not.
        assert_eq!(value["file_comparators"], json!({"rules": []}));
        assert_eq!(value["disk_io_concurrency"], json!({"mode": "automatic"}));
        assert!(value.get("scroll_glide").is_none());
        assert_eq!(decoded(&value).unwrap(), preferences, "{choice:?}");
    }
    // With a glide choice too, both are written.
    let both = UserPreferences {
        scroll_glide: ScrollGlide::Off,
        ..speed(WheelSpeed::VerySlow)
    };
    let value = stored(&both);
    assert_eq!(value["version"], 38);
    assert_eq!(value["scroll_glide"], 0);
    assert_eq!(value["wheel_speed"], 50);
    assert_eq!(decoded(&value).unwrap(), both);
    // Choosing Medium again restores the previous minimum schema.
    let value = stored(&speed(WheelSpeed::Medium));
    assert_eq!(value["version"], 10);
}

#[test]
fn the_speed_survives_settings_export_and_import() {
    for choice in [WheelSpeed::VerySlow, WheelSpeed::VeryFast] {
        let file = encode_settings_file(speed(choice)).unwrap();
        let imported = decode_settings_file(&file).unwrap();
        assert_eq!(imported.preferences.wheel_speed, choice);
    }
}

#[test]
fn schema_38_fields_are_versioned_strict_and_future_versions_stay_refused() {
    let valid = stored(&speed(WheelSpeed::Fast));
    assert!(decoded(&valid).is_ok());

    // A schema 38 document needs its own field.
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("wheel_speed");
    assert!(decoded(&missing).is_err());

    // Earlier schemas cannot carry the speed.
    for version in [21, 34, 37] {
        let mut premature = valid.clone();
        premature["version"] = version.into();
        premature["glyph_fitting"] = "snapped".into();
        premature["scroll_glide"] = 200.into();
        assert!(decoded(&premature).is_err(), "version {version}");
    }

    for invalid in [
        Value::Null,
        json!(-1),
        json!(1.5),
        json!(0),
        json!(125),
        json!(70000),
        json!("fast"),
    ] {
        let mut mutated = valid.clone();
        mutated["wheel_speed"] = invalid.clone();
        assert!(decoded(&mutated).is_err(), "{invalid}");
    }

    let mut future = valid;
    future["version"] = 41.into();
    let error = decoded(&future).unwrap_err();
    assert_eq!(error.to_string(), "unsupported preferences version");
}
