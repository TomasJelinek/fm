use midnight_platform_common::settings::{decode_preferences, encode_preferences};

fn current() -> serde_json::Value {
    serde_json::json!({
        "version": 5,
        "theme": "dark",
        "density": "desktop",
        "windows_shell": "powershell",
        "font_family": "文档 e\u{301}",
        "desktop_font_size": 11,
        "touch_font_size": 15,
        "bookmarks": [],
        "quick_search_filter": true,
        "size_display": "bytes",
        "remember_command_history": true,
        "command_history": [
            {"command": "  echo 文档 👩🏽‍💻  ", "shell": "powershell"},
            {"command": "dir", "shell": "cmd"}
        ]
    })
}

#[test]
fn exact_command_text_shell_and_size_preferences_roundtrip() {
    let input = current();
    let decoded = decode_preferences(&serde_json::to_vec(&input).unwrap()).unwrap();
    let output: serde_json::Value =
        serde_json::from_slice(&encode_preferences(decoded).unwrap()).unwrap();
    let mut expected = input;
    expected["version"] = 8.into();
    expected["shortcuts"] = serde_json::json!([]);
    expected["panel_filters"] = serde_json::json!([{ "enabled": false, "mode": "masks", "expression": "*.*", "case_sensitive": false }, { "enabled": false, "mode": "masks", "expression": "*.*", "case_sensitive": false }]);
    expected["tabs"] = serde_json::json!([
        {"paths": [], "active": 0}, {"paths": [], "active": 0}
    ]);
    expected["panel_view_modes"] = serde_json::json!(["detailed", "detailed"]);
    assert_eq!(expected, output);
}

#[test]
fn history_optout_omits_session_commands_from_persisted_data() {
    let mut input = current();
    input["remember_command_history"] = false.into();
    let decoded = decode_preferences(&serde_json::to_vec(&input).unwrap()).unwrap();
    let output: serde_json::Value =
        serde_json::from_slice(&encode_preferences(decoded).unwrap()).unwrap();
    assert_eq!(output["command_history"], serde_json::json!([]));
    assert_eq!(output["remember_command_history"], false);
}

#[test]
fn version_four_migrates_with_readable_sizes_empty_persistent_history() {
    let mut input = current();
    input["version"] = 4.into();
    for key in [
        "size_display",
        "remember_command_history",
        "command_history",
    ] {
        input.as_object_mut().unwrap().remove(key);
    }
    let decoded = decode_preferences(&serde_json::to_vec(&input).unwrap()).unwrap();
    let output: serde_json::Value =
        serde_json::from_slice(&encode_preferences(decoded).unwrap()).unwrap();
    assert_eq!(output["version"], 8);
    assert_eq!(output["size_display"], "readable");
    assert_eq!(output["remember_command_history"], true);
    assert_eq!(output["command_history"], serde_json::json!([]));
    assert_eq!(output["quick_search_filter"], true);
    assert_eq!(output["font_family"], "文档 e\u{301}");
}

#[test]
fn malformed_and_duplicate_command_history_is_rejected_without_truncation() {
    for key in [
        "size_display",
        "remember_command_history",
        "command_history",
    ] {
        let mut input = current();
        input.as_object_mut().unwrap().remove(key);
        assert!(decode_preferences(&serde_json::to_vec(&input).unwrap()).is_err());
        input[key] = serde_json::Value::Null;
        assert!(decode_preferences(&serde_json::to_vec(&input).unwrap()).is_err());
    }
    for history in [
        serde_json::json!([{"command": " ", "shell": "cmd"}]),
        serde_json::json!([{"command": "echo\u{0}x", "shell": "posix"}]),
        serde_json::json!([{"command": "echo", "shell": "unknown"}]),
        serde_json::json!([
            {"command": "echo", "shell": "cmd"},
            {"command": "echo", "shell": "powershell"}
        ]),
    ] {
        let mut input = current();
        input["command_history"] = history;
        assert!(decode_preferences(&serde_json::to_vec(&input).unwrap()).is_err());
    }
    let mut legacy = current();
    legacy["version"] = 4.into();
    assert!(decode_preferences(&serde_json::to_vec(&legacy).unwrap()).is_err());
}
