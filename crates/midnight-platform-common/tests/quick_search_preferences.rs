use midnight_platform_common::settings::{decode_preferences, encode_preferences};

fn version_three() -> serde_json::Value {
    serde_json::json!({
        "version": 3,
        "theme": "dark",
        "density": "touch",
        "windows_shell": "cmd",
        "font_family": "文档 e\u{301}",
        "desktop_font_size": 12,
        "touch_font_size": 17,
        "bookmarks": []
    })
}

#[test]
fn legacy_options_migrate_with_filter_disabled_and_existing_choices_preserved() {
    let preferences = decode_preferences(&serde_json::to_vec(&version_three()).unwrap()).unwrap();
    let stored: serde_json::Value =
        serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
    assert_eq!(stored["version"], 6);
    assert_eq!(stored["quick_search_filter"], false);
    assert_eq!(stored["theme"], "dark");
    assert_eq!(stored["density"], "touch");
    assert_eq!(stored["windows_shell"], "cmd");
    assert_eq!(stored["font_family"], "文档 e\u{301}");
    assert_eq!(stored["desktop_font_size"], 12);
    assert_eq!(stored["touch_font_size"], 17);
}

#[test]
fn filter_preference_roundtrips_both_values_in_current_schema() {
    for enabled in [false, true] {
        let mut input = version_three();
        input["version"] = 4.into();
        input["quick_search_filter"] = enabled.into();
        let decoded = decode_preferences(&serde_json::to_vec(&input).unwrap()).unwrap();
        let output: serde_json::Value =
            serde_json::from_slice(&encode_preferences(decoded).unwrap()).unwrap();
        assert_eq!(output["quick_search_filter"], enabled);
        assert_eq!(output["font_family"], "文档 e\u{301}");
    }
}

#[test]
fn filter_setting_rejects_missing_null_wrong_type_duplicates_and_older_version_fields() {
    let mut input = version_three();
    input["version"] = 4.into();
    assert!(decode_preferences(&serde_json::to_vec(&input).unwrap()).is_err());
    for malformed in [
        serde_json::Value::Null,
        "true".into(),
        1.into(),
        vec![true].into(),
    ] {
        input["quick_search_filter"] = malformed;
        assert!(decode_preferences(&serde_json::to_vec(&input).unwrap()).is_err());
    }
    input["quick_search_filter"] = true.into();
    let serialized = serde_json::to_string(&input).unwrap();
    let duplicate = format!(
        "{},\"quick_search_filter\":false}}",
        &serialized[..serialized.len() - 1]
    );
    assert!(decode_preferences(duplicate.as_bytes()).is_err());
    for version in [1, 2, 3] {
        let mut legacy = version_three();
        legacy["version"] = version.into();
        legacy["quick_search_filter"] = true.into();
        assert!(decode_preferences(&serde_json::to_vec(&legacy).unwrap()).is_err());
    }
}
