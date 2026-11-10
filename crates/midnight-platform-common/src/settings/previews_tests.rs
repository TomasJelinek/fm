use super::{decode_preferences, encode_preferences};
use midnight_core::UserPreferences;

#[test]
fn preview_defaults_preserve_legacy_encoding_and_nondefaults_roundtrip_in_v12() {
    let encoded = encode_preferences(UserPreferences::default()).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(document["version"], 10);
    assert!(document.get("previews").is_none());
    assert!(decode_preferences(&encoded).unwrap().thumbnails_enabled);
    assert!(!decode_preferences(&encoded).unwrap().thumbnails_network);
    for enabled in [false, true] {
        for network in [false, true] {
            let preferences = UserPreferences {
                thumbnails_enabled: enabled,
                thumbnails_network: network,
                ..Default::default()
            };
            preferences.validate().unwrap();
            let encoded = encode_preferences(preferences.clone()).unwrap();
            let document: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(
                document["version"],
                if enabled && !network { 10 } else { 12 }
            );
            assert_eq!(decode_preferences(&encoded).unwrap(), preferences);
        }
    }
}

#[test]
fn preview_schema_rejects_missing_null_wrong_type_and_old_version_fields() {
    let encoded = encode_preferences(UserPreferences {
        thumbnails_network: true,
        ..Default::default()
    })
    .unwrap();
    let document: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    for previews in [
        serde_json::Value::Null,
        serde_json::json!({"enabled":true}),
        serde_json::json!({"enabled":"true","network":false}),
        serde_json::json!({"enabled":true,"network":false,"extra":1}),
    ] {
        let mut changed = document.clone();
        changed["previews"] = previews;
        assert!(decode_preferences(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
    let mut missing = document.clone();
    missing.as_object_mut().unwrap().remove("previews");
    assert!(decode_preferences(&serde_json::to_vec(&missing).unwrap()).is_err());
    let mut old = document;
    old["version"] = 10.into();
    assert!(decode_preferences(&serde_json::to_vec(&old).unwrap()).is_err());
}

#[test]
fn thumbnail_size_migrates_legacy_settings_and_roundtrips_each_supported_size() {
    let legacy = encode_preferences(UserPreferences::default()).unwrap();
    assert_eq!(decode_preferences(&legacy).unwrap().thumbnail_size, 72);
    for size in [48, 64, 72, 96, 128, 192, 256] {
        let preferences = UserPreferences {
            thumbnail_size: size,
            ..Default::default()
        };
        let bytes = encode_preferences(preferences.clone()).unwrap();
        assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["version"], if size == 72 { 10 } else { 25 });
    }
}

#[test]
fn thumbnail_size_rejects_invalid_values_and_fields_in_older_schemas() {
    let preferences = UserPreferences {
        thumbnail_size: 192,
        ..Default::default()
    };
    let mut document: serde_json::Value =
        serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
    for value in [
        serde_json::Value::Null,
        serde_json::json!(0),
        serde_json::json!(47),
        serde_json::json!(73),
        serde_json::json!(257),
        serde_json::json!("192"),
    ] {
        let mut invalid = document.clone();
        invalid["thumbnail_size"] = value;
        assert!(decode_preferences(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    document["version"] = 24.into();
    assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
    document["version"] = 25.into();
    document.as_object_mut().unwrap().remove("thumbnail_size");
    assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
}
