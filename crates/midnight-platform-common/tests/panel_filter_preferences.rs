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
