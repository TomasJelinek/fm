use super::{decode_preferences, encode_preferences};
use midnight_core::{
    ShortcutBinding, ShortcutChord, ShortcutContext, ShortcutMap, UserPreferences,
};

#[test]
fn shortcut_overrides_survive_roundtrip_with_disabled_actions_and_unicode_keys() {
    let preferences = UserPreferences {
        shortcuts: vec![
            ShortcutBinding {
                action: "files.rename".into(),
                keys: vec!["Ctrl+文".into()],
            },
            ShortcutBinding {
                action: "files.trash".into(),
                keys: vec![],
            },
        ],
        ..UserPreferences::default()
    };
    let bytes = encode_preferences(preferences.clone()).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stored["version"], 10);
    assert_eq!(stored["shortcuts"][0]["keys"][0], "Ctrl+文");
    let restored = decode_preferences(&bytes).unwrap();
    assert_eq!(restored, preferences);
    let map = ShortcutMap::new(&restored.shortcuts).unwrap();
    assert_eq!(
        map.resolve(
            &ShortcutChord::parse("Ctrl+文").unwrap(),
            ShortcutContext::Files
        )
        .unwrap()
        .id,
        "files.rename"
    );
    for removed in ["F2", "Delete", "F8"] {
        assert!(
            map.resolve(
                &ShortcutChord::parse(removed).unwrap(),
                ShortcutContext::Files
            )
            .is_none()
        );
    }
}

#[test]
fn version_six_migrates_to_default_keys_and_seven_requires_explicit_overrides_array() {
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&encode_preferences(UserPreferences::default()).unwrap()).unwrap();
    legacy["version"] = 6.into();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("disabled_viewer_plugins");
    legacy.as_object_mut().unwrap().remove("editor");
    legacy.as_object_mut().unwrap().remove("shortcuts");
    legacy.as_object_mut().unwrap().remove("panel_filters");
    let preferences = decode_preferences(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(preferences.shortcuts.is_empty());
    assert_eq!(
        ShortcutMap::new(&preferences.shortcuts)
            .unwrap()
            .resolve(
                &ShortcutChord::parse("Ctrl+C").unwrap(),
                ShortcutContext::Files
            )
            .unwrap()
            .id,
        "files.copy"
    );
    legacy["version"] = 7.into();
    assert!(decode_preferences(&serde_json::to_vec(&legacy).unwrap()).is_err());
    legacy["shortcuts"] = serde_json::json!([]);
    assert!(decode_preferences(&serde_json::to_vec(&legacy).unwrap()).is_ok());
    legacy["version"] = 6.into();
    assert!(decode_preferences(&serde_json::to_vec(&legacy).unwrap()).is_err());
}

#[test]
fn shortcut_codec_rejects_invalid_data_without_partially_applying() {
    let defaults: serde_json::Value =
        serde_json::from_slice(&encode_preferences(UserPreferences::default()).unwrap()).unwrap();
    for invalid in [
        serde_json::Value::Null,
        serde_json::json!({}),
        serde_json::json!([{"action":"files.rename","keys":["Ctrl+C"]}]),
        serde_json::json!([{"action":"files.rename","keys":["F99"]}]),
        serde_json::json!([{"action":"unknown","keys":[]}]),
        serde_json::json!([{"action":"files.rename","keys":[],"unknown":true}]),
        serde_json::json!([{"action":"files.rename","keys":[]},{"action":"files.rename","keys":[]}]),
    ] {
        let mut input = defaults.clone();
        input["shortcuts"] = invalid;
        assert!(
            decode_preferences(&serde_json::to_vec(&input).unwrap()).is_err(),
            "{input}"
        );
    }
    let invalid = UserPreferences {
        shortcuts: vec![ShortcutBinding {
            action: "files.rename".into(),
            keys: vec!["Ctrl+C".into()],
        }],
        ..UserPreferences::default()
    };
    assert!(invalid.validate().is_err());
    assert!(encode_preferences(invalid).is_err());
}
