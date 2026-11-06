use midnight_core::{EditorPreferences, UserPreferences};
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

#[test]
fn editor_literal_arguments_and_native_executable_roundtrip() {
    let preferences = UserPreferences {
        editor: EditorPreferences {
            executable: Some(
                std::env::current_dir()
                    .unwrap()
                    .join("Editor 文档 & tools/editor.exe"),
            ),
            arguments: vec![
                "{file}".into(),
                "--label=文档 ; $HOME ' \"".into(),
                "".into(),
            ],
        },
        ..Default::default()
    };
    let bytes = encode_preferences(preferences.clone()).unwrap();
    assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
}

#[test]
fn previous_settings_migrate_to_default_editor_and_new_version_requires_editor() {
    let mut json: serde_json::Value =
        serde_json::from_slice(&encode_preferences(UserPreferences::default()).unwrap()).unwrap();
    assert_eq!(json["version"], 10);
    json.as_object_mut().unwrap().remove("editor");
    assert!(decode_preferences(&serde_json::to_vec(&json).unwrap()).is_err());
    json["version"] = 8.into();
    json.as_object_mut()
        .unwrap()
        .remove("disabled_viewer_plugins");
    assert_eq!(
        decode_preferences(&serde_json::to_vec(&json).unwrap())
            .unwrap()
            .editor,
        EditorPreferences::default()
    );
}
