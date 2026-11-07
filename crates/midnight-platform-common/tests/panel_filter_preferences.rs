use midnight_core::UserPreferences;
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

fn current() -> serde_json::Value {
    serde_json::from_slice(&encode_preferences(UserPreferences::default()).unwrap()).unwrap()
}

#[test]
fn independent_mask_and_regex_filters_roundtrip_in_current_schema() {
    let mut value = current();
    value["version"] = 10.into();
    value["panel_filters"] = serde_json::json!([
        {"enabled":true,"mode":"masks","expression":"*.txt|secret*","case_sensitive":false},
        {"enabled":true,"mode":"regex","expression":"^文[0-9]+$","case_sensitive":true}
    ]);
    let loaded = decode_preferences(&serde_json::to_vec(&value).unwrap()).unwrap();
    let stored: serde_json::Value =
        serde_json::from_slice(&encode_preferences(loaded).unwrap()).unwrap();
    assert_eq!(stored, value);
}

#[test]
fn version_seven_migrates_with_disabled_filters_and_eight_requires_them() {
    let mut legacy = current();
    legacy["version"] = 7.into();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("disabled_viewer_plugins");
    legacy.as_object_mut().unwrap().remove("editor");
    legacy.as_object_mut().unwrap().remove("panel_filters");
    let loaded = decode_preferences(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    let stored: serde_json::Value =
        serde_json::from_slice(&encode_preferences(loaded).unwrap()).unwrap();
    assert_eq!(stored["version"], 10);
    assert_eq!(stored["panel_filters"][0]["enabled"], false);
    assert_eq!(stored["panel_filters"][1]["enabled"], false);
    legacy["version"] = 8.into();
    assert!(decode_preferences(&serde_json::to_vec(&legacy).unwrap()).is_err());
}

#[test]
fn malformed_filters_are_rejected_instead_of_partially_loading() {
    let original = current();
    for expression in ["[".to_owned(), "a".repeat(4097)] {
        let mut value = original.clone();
        value["version"] = 10.into();
        value["panel_filters"] = serde_json::json!([
            {"enabled":true,"mode":"regex","expression":expression,"case_sensitive":false},
            {"enabled":false,"mode":"masks","expression":"*.*","case_sensitive":false}
        ]);
        assert!(decode_preferences(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[test]
fn metadata_expression_roundtrip_uses_schema18_and_preserves_required_earlier_defaults() {
    let mut preferences = UserPreferences::default();
    preferences.panel_filters[0] = midnight_core::PanelFilter {
        enabled: true,
        mode: midnight_core::FilterMode::Expression,
        expression: "kind = file and size >= 10MiB".into(),
        case_sensitive: false,
    };
    let bytes = encode_preferences(preferences.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["version"], 18);
    assert_eq!(value["user_commands"], serde_json::json!([]));
    assert!(value.get("checksum_algorithms").is_some());
    assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
}

#[test]
fn expression_schema_preserves_commands_checksums_and_rejects_old_or_missing_required_fields() {
    use midnight_core::*;
    use std::sync::Arc;
    let mut preferences = UserPreferences::default();
    preferences.panel_filters[1] = PanelFilter {
        enabled: true,
        mode: FilterMode::Expression,
        expression: "modified >= \"2026-11-01T00:00:00Z\"".into(),
        case_sensitive: true,
    };
    preferences.checksum_algorithms =
        ChecksumAlgorithms::new(&[ChecksumAlgorithm::Md5, ChecksumAlgorithm::Sha512]).unwrap();
    preferences.user_commands = Arc::new(vec![UserCommand {
        id: UserCommandId::new("kept").unwrap(),
        name: "Kept 文档".into(),
        enabled: false,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: vec![],
        },
    }]);
    let bytes = encode_preferences(preferences.clone()).unwrap();
    assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut old = value.clone();
    old["version"] = 17.into();
    assert!(decode_preferences(&serde_json::to_vec(&old).unwrap()).is_err());
    for field in ["user_commands", "checksum_algorithms"] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            decode_preferences(&serde_json::to_vec(&missing).unwrap()).is_err(),
            "{field}"
        );
    }
    for enabled in [true, false] {
        let mut invalid = preferences.clone();
        invalid.panel_filters[1].enabled = enabled;
        invalid.panel_filters[1].expression = "size = \"large\"".into();
        assert!(encode_preferences(invalid).is_err());
    }
    let mut colors = value;
    colors["filename_colors"][0]["mode"] = "expression".into();
    colors["filename_colors"][0]["expression"] = "kind = file".into();
    assert!(decode_preferences(&serde_json::to_vec(&colors).unwrap()).is_err());
}
