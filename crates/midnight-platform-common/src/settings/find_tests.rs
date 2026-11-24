use super::{decode_preferences, decode_settings_file, encode_preferences, encode_settings_file};
use midnight_core::{FindPreferences, UserPreferences, WindowGeometry, WindowRect};
use serde_json::{Value, json};

fn preferences() -> UserPreferences {
    let mut preferences = UserPreferences::default();
    preferences.find.mask_history = vec!["文档*.txt".into(), "*.rs;*.slint".into()];
    preferences.find.window_geometry = Some(WindowGeometry {
        normal: WindowRect {
            x: -1200,
            y: 80,
            width: 940,
            height: 650,
        },
        monitor: None,
        maximized: false,
        fullscreen: false,
    });
    preferences
}

fn stored(preferences: UserPreferences) -> Value {
    serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap()
}

fn decode(value: Value) -> std::io::Result<UserPreferences> {
    decode_preferences(&serde_json::to_vec(&value).unwrap())
}

#[test]
fn find_history_and_placement_round_trip_in_schema_41_and_settings_exchange() {
    let preferences = preferences();
    let value = stored(preferences.clone());
    assert_eq!(value["version"], 41);
    assert_eq!(
        value["find"]["mask_history"],
        json!(["文档*.txt", "*.rs;*.slint"])
    );
    assert_eq!(decode(value).unwrap(), preferences);
    let exported = encode_settings_file(preferences.clone()).unwrap();
    assert_eq!(
        decode_settings_file(&exported).unwrap().preferences,
        preferences
    );
}

#[test]
fn previous_settings_keep_find_defaults_and_defaults_keep_the_legacy_schema() {
    let defaults = stored(UserPreferences::default());
    assert_eq!(defaults["version"], 10);
    assert!(defaults.get("find").is_none());
    assert_eq!(decode(defaults).unwrap().find, FindPreferences::default());
    let mut previous = UserPreferences::default();
    previous.updates.check_at_startup = false;
    let previous = stored(previous);
    assert_eq!(previous["version"], 40);
    assert_eq!(decode(previous).unwrap().find, FindPreferences::default());
}

#[test]
fn malformed_find_history_placement_and_schema_fields_are_refused() {
    let valid = stored(preferences());
    for invalid in [
        Value::Null,
        json!({}),
        json!({"mask_history": null}),
        json!({"mask_history": ["*.rs", "*.rs"]}),
        json!({"mask_history": [""]}),
        json!({"mask_history": ["bad\0mask"]}),
        json!({"mask_history": ["x".repeat(4097)]}),
        json!({"mask_history": (0..31).map(|i| i.to_string()).collect::<Vec<_>>()}),
        json!({"mask_history": [], "unknown": true}),
        json!({"mask_history": [], "window_geometry": null}),
    ] {
        let mut value = valid.clone();
        value["find"] = invalid;
        assert!(decode(value).is_err());
    }
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("find");
    assert!(decode(missing).is_err());
    let mut invalid_geometry = valid.clone();
    invalid_geometry["find"]["window_geometry"]["normal"]["width"] = json!(0);
    assert!(decode(invalid_geometry).is_err());
    let mut old = stored(UserPreferences::default());
    old["find"] = valid["find"].clone();
    assert!(decode(old).is_err());
}
