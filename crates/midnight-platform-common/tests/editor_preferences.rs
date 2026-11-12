use midnight_core::{EditorPreferences, EditorRule, FilterMode, UserPreferences};
use midnight_platform_common::settings::{decode_preferences, encode_preferences};
use serde_json::{Value, json};

fn configured() -> UserPreferences {
    let root = std::env::current_dir().unwrap();
    UserPreferences {
        editor: EditorPreferences {
            executable: Some(root.join("Editor 文档 & tools/editor.exe")),
            arguments: vec![
                "{file}".into(),
                "--label=文档 ; $HOME ' \"".into(),
                "".into(),
            ],
            rules: vec![
                EditorRule {
                    expression: "*.txt;*.md|secret*".into(),
                    executable: Some(root.join("Markdown 编辑器/editor.exe")),
                    arguments: vec!["--wait".into(), "{file}".into(), "two words".into()],
                    working_directory: Some(root.join("文档 & workspace")),
                    ..Default::default()
                },
                EditorRule {
                    enabled: false,
                    mode: FilterMode::Regex,
                    expression: r"^README(?:\..*)?$".into(),
                    case_sensitive: true,
                    ..Default::default()
                },
            ],
        },
        ..Default::default()
    }
}

#[test]
fn ordered_editor_rules_literals_native_paths_and_disabled_state_roundtrip() {
    let preferences = configured();
    let bytes = encode_preferences(preferences.clone()).unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["version"], 30);
    assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
}

#[test]
fn previous_settings_preserve_custom_editor_as_fallback_and_old_defaults_still_load() {
    let preferences = configured();
    let mut json: Value =
        serde_json::from_slice(&encode_preferences(preferences.clone()).unwrap()).unwrap();
    json["version"] = 29.into();
    json["editor"].as_object_mut().unwrap().remove("rules");
    json.as_object_mut().unwrap().remove("panel_filter_history");
    let migrated = decode_preferences(&serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(migrated.editor.executable, preferences.editor.executable);
    assert_eq!(migrated.editor.arguments, preferences.editor.arguments);
    assert!(migrated.editor.rules.is_empty());
    let oldest =
        decode_preferences(br#"{"version":1,"theme":"blue","density":"desktop"}"#).unwrap();
    assert_eq!(oldest.editor, EditorPreferences::default());
}

#[test]
fn versioned_rules_reject_missing_null_premature_invalid_and_unknown_data() {
    let valid: Value = serde_json::from_slice(&encode_preferences(configured()).unwrap()).unwrap();
    let mut missing = valid.clone();
    missing["editor"].as_object_mut().unwrap().remove("rules");
    assert!(decode_preferences(&serde_json::to_vec(&missing).unwrap()).is_err());
    for rules in [Value::Null, json!({}), json!([{}])] {
        let mut document = valid.clone();
        document["editor"]["rules"] = rules;
        assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
    }
    let mut premature = valid.clone();
    premature["version"] = 29.into();
    premature
        .as_object_mut()
        .unwrap()
        .remove("panel_filter_history");
    assert!(decode_preferences(&serde_json::to_vec(&premature).unwrap()).is_err());
    for (field, value) in [
        ("mode", json!("expression")),
        ("expression", json!("[")),
        ("unknown", json!(true)),
        ("working_directory", json!("relative/folder")),
        ("arguments", json!(["missing file placeholder"])),
    ] {
        let mut document = valid.clone();
        document["editor"]["rules"][1][field] = value;
        assert!(
            decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err(),
            "accepted {field}"
        );
    }
}

#[test]
fn encoder_validates_disabled_regex_before_publication() {
    let mut preferences = configured();
    preferences.editor.rules[1].expression = "[".into();
    let error = encode_preferences(preferences).unwrap_err().to_string();
    assert!(error.contains("Editor rule 2:"), "{error}");
}
