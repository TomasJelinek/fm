use midnight_core::UserPreferences;
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

#[test]
fn disabled_plugins_roundtrip_and_older_preferences_enable_available_plugins() {
    let preferences = UserPreferences {
        disabled_viewer_plugins: vec!["midnight.viewer.source".into()],
        ..Default::default()
    };
    let bytes = encode_preferences(preferences.clone()).unwrap();
    assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
    let mut legacy: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    legacy["version"] = 9.into();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("disabled_viewer_plugins");
    assert!(
        decode_preferences(&serde_json::to_vec(&legacy).unwrap())
            .unwrap()
            .disabled_viewer_plugins
            .is_empty()
    );
}

#[test]
fn duplicate_invalid_or_oversized_plugin_preferences_are_rejected() {
    for ids in [
        vec!["a".into(), "a".into()],
        vec!["bad/id".into()],
        vec!["x".repeat(129)],
        (0..65).map(|index| format!("viewer.{index}")).collect(),
    ] {
        let preferences = UserPreferences {
            disabled_viewer_plugins: ids,
            ..Default::default()
        };
        assert!(encode_preferences(preferences).is_err());
    }
}
