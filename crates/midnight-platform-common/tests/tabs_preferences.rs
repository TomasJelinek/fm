use midnight_platform_common::settings::{decode_preferences, encode_preferences};

fn saved() -> serde_json::Value {
    let root = std::env::current_dir().unwrap();
    serde_json::json!({
        "version": 6, "theme": "dark", "density": "desktop", "windows_shell": "powershell",
        "font_family": "", "desktop_font_size": 11, "touch_font_size": 15, "bookmarks": [],
        "quick_search_filter": false, "size_display": "readable", "remember_command_history": true,
        "command_history": [], "panel_view_modes": ["detailed", "detailed"],
        "tabs": [
            {"paths": [root.join("文档 e\u{301} 🧪"), root.join("second")], "active": 1},
            {"paths": [root.join("right")], "active": 0}
        ]
    })
}

#[test]
fn ordered_native_tab_paths_and_active_indices_roundtrip() {
    let input = saved();
    let decoded = decode_preferences(&serde_json::to_vec(&input).unwrap()).unwrap();
    let output: serde_json::Value =
        serde_json::from_slice(&encode_preferences(decoded).unwrap()).unwrap();
    assert_eq!(output, input);
}

#[test]
fn legacy_preferences_migrate_without_inventing_saved_tab_locations() {
    let mut legacy = saved();
    legacy["version"] = 5.into();
    legacy.as_object_mut().unwrap().remove("tabs");
    legacy.as_object_mut().unwrap().remove("panel_view_modes");
    let decoded = decode_preferences(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    let output: serde_json::Value =
        serde_json::from_slice(&encode_preferences(decoded).unwrap()).unwrap();
    assert_eq!(output["version"], 6);
    assert_eq!(
        output["tabs"],
        serde_json::json!([
            {"paths": [], "active": 0}, {"paths": [], "active": 0}
        ])
    );
}

#[test]
fn malformed_tab_sessions_are_rejected_without_truncation() {
    let valid = saved();
    for tabs in [
        serde_json::Value::Null,
        serde_json::json!([]),
        serde_json::json!([{"paths": [], "active": 0}]),
        serde_json::json!([{"paths": [], "active": 1}, {"paths": [], "active": 0}]),
        serde_json::json!([{"paths": ["relative"], "active": 0}, {"paths": [], "active": 0}]),
        serde_json::json!([{"paths": ["/nul\u{0}path"], "active": 0}, {"paths": [], "active": 0}]),
    ] {
        let mut value = valid.clone();
        value["tabs"] = tabs;
        assert!(decode_preferences(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    for (key, value) in [
        ("active", serde_json::json!(2)),
        ("unexpected", serde_json::json!(true)),
        (
            "paths",
            serde_json::json!(vec![std::env::current_dir().unwrap(); 33]),
        ),
    ] {
        let mut input = valid.clone();
        input["tabs"][0][key] = value;
        assert!(decode_preferences(&serde_json::to_vec(&input).unwrap()).is_err());
    }
    let mut missing = valid;
    missing.as_object_mut().unwrap().remove("tabs");
    assert!(decode_preferences(&serde_json::to_vec(&missing).unwrap()).is_err());
}
