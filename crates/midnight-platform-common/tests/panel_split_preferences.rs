use midnight_core::UserPreferences;
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

fn stored_split(value: serde_json::Value) -> serde_json::Value {
    let mut preferences = UserPreferences::default();
    preferences.viewer.wrap = true;
    let mut json: serde_json::Value =
        serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
    json["version"] = 23.into();
    json["panel_split_basis_points"] = value;
    json
}

#[test]
fn splitter_ratio_survives_restart_with_other_viewer_choices() {
    for ratio in [1, 2500, 6180, 9999] {
        let json = stored_split(ratio.into());
        let preferences = decode_preferences(&serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(preferences.viewer.wrap);
        let saved: serde_json::Value =
            serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
        assert_eq!(saved["panel_split_basis_points"], ratio);
        assert_eq!(saved["version"], 23);
    }
}

#[test]
fn invalid_split_ratios_missing_values_and_schema_downgrades_are_rejected() {
    for ratio in [
        serde_json::Value::Null,
        0.into(),
        10_000.into(),
        (-1).into(),
        65_536.into(),
        0.5.into(),
        "5000".into(),
    ] {
        assert!(decode_preferences(&serde_json::to_vec(&stored_split(ratio)).unwrap()).is_err());
    }
    let mut json = stored_split(6100.into());
    json["version"] = 21.into();
    assert!(decode_preferences(&serde_json::to_vec(&json).unwrap()).is_err());
    json["version"] = 23.into();
    json.as_object_mut()
        .unwrap()
        .remove("panel_split_basis_points");
    assert!(decode_preferences(&serde_json::to_vec(&json).unwrap()).is_err());
}

#[test]
fn legacy_preferences_restore_center_and_custom_split_keeps_comparator_font() {
    let old = encode_preferences(UserPreferences::default()).unwrap();
    assert_eq!(
        decode_preferences(&old).unwrap().panel_split_basis_points,
        5000
    );
    let mut preferences = UserPreferences {
        panel_split_basis_points: 3700,
        ..Default::default()
    };
    preferences.viewer.comparison_font = "Cascadia Mono".into();
    let saved = encode_preferences(preferences.clone()).unwrap();
    assert_eq!(decode_preferences(&saved).unwrap(), preferences);
    preferences.panel_split_basis_points = 5000;
    let centered = encode_preferences(preferences.clone()).unwrap();
    assert_eq!(decode_preferences(&centered).unwrap(), preferences);
    let json: serde_json::Value = serde_json::from_slice(&centered).unwrap();
    assert_eq!(json["version"], 22);
    assert!(json.get("panel_split_basis_points").is_none());
}
