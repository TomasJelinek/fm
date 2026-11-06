use midnight_platform_common::settings::{decode_preferences, encode_preferences};

fn current() -> serde_json::Value {
    serde_json::json!({
        "version": 6, "theme": "blue", "density": "desktop", "windows_shell": "powershell",
        "font_family": "", "desktop_font_size": 11, "touch_font_size": 15,
        "bookmarks": [], "quick_search_filter": false, "size_display": "readable",
        "remember_command_history": true, "command_history": [],
        "tabs": [{"paths":[],"active":0},{"paths":[],"active":0}],
        "panel_view_modes": ["brief", "thumbnails"]
    })
}

#[test]
fn independent_panel_modes_roundtrip_as_stable_names() {
    let input = current();
    let loaded = decode_preferences(&serde_json::to_vec(&input).unwrap()).unwrap();
    let output: serde_json::Value =
        serde_json::from_slice(&encode_preferences(loaded).unwrap()).unwrap();
    let mut migrated = input;
    migrated["version"] = 8.into();
    migrated["shortcuts"] = serde_json::json!([]);
    migrated["panel_filters"] = serde_json::json!([{ "enabled": false, "mode": "masks", "expression": "*.*", "case_sensitive": false }, { "enabled": false, "mode": "masks", "expression": "*.*", "case_sensitive": false }]);
    assert_eq!(output, migrated);
}

#[test]
fn legacy_settings_default_both_panels_to_detailed() {
    let mut input = current();
    input["version"] = 5.into();
    input.as_object_mut().unwrap().remove("panel_view_modes");
    input.as_object_mut().unwrap().remove("tabs");
    let loaded = decode_preferences(&serde_json::to_vec(&input).unwrap()).unwrap();
    let output: serde_json::Value =
        serde_json::from_slice(&encode_preferences(loaded).unwrap()).unwrap();
    assert_eq!(
        output["panel_view_modes"],
        serde_json::json!(["detailed", "detailed"])
    );
}

#[test]
fn malformed_panel_modes_are_rejected() {
    for modes in [
        serde_json::json!(["unknown", "detailed"]),
        serde_json::json!(["brief"]),
        serde_json::json!([0, 1]),
        serde_json::Value::Null,
    ] {
        let mut input = current();
        input["panel_view_modes"] = modes;
        assert!(decode_preferences(&serde_json::to_vec(&input).unwrap()).is_err());
    }
    let mut input = current();
    input.as_object_mut().unwrap().remove("panel_view_modes");
    assert!(decode_preferences(&serde_json::to_vec(&input).unwrap()).is_err());
}
