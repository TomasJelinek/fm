use super::{decode_preferences, encode_preferences};
use midnight_core::UserPreferences;
use serde_json::{Value, json};

#[cfg(unix)]
#[path = "file_list_unix_tests.rs"]
mod unix;
#[cfg(windows)]
#[path = "file_list_windows_tests.rs"]
mod windows;

fn base_document() -> Value {
    let mut preferences = UserPreferences::default();
    preferences
        .file_list
        .remember("$(FileName)$(TAB)$(FileSize)$(CRLF)")
        .unwrap();
    let document: Value =
        serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
    assert_eq!(document["version"], 26);
    document
}

fn file_list() -> Value {
    json!({
        "template_history": ["$(FileName)$(TAB)$(FileSize)$(CRLF)", "$(FileName)$(CRLF)"],
        "destination": 2,
        "path": std::env::temp_dir().join("資料 file list.txt"),
        "append": true
    })
}

#[test]
fn file_list_version_26_roundtrips_history_unicode_destination_and_existing_choices() {
    let mut document = base_document();
    document["version"] = 26.into();
    document["thumbnail_size"] = 96.into();
    document["file_list"] = file_list();
    let decoded = decode_preferences(&serde_json::to_vec(&document).unwrap())
        .expect("version 26 Make File List preferences");
    assert_eq!(decoded.thumbnail_size, 96);
    let encoded = encode_preferences(decoded.clone()).unwrap();
    let roundtrip: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(roundtrip["version"], 26);
    assert_eq!(roundtrip["file_list"], document["file_list"]);
    assert_eq!(decode_preferences(&encoded).unwrap(), decoded);
}

#[test]
fn file_list_strict_schema_rejects_missing_null_malformed_and_premature_fields() {
    let mut valid = base_document();
    valid["version"] = 26.into();
    valid["thumbnail_size"] = 96.into();
    valid["file_list"] = file_list();
    for value in [
        Value::Null,
        json!({}),
        json!({"template_history": [], "destination": "clipboard", "append": false}),
        json!({"template_history": [], "destination": 3, "append": false}),
        json!({"template_history": [], "destination": 0, "path": null, "append": false}),
        json!({"template_history": [], "destination": 0, "append": false, "unknown": 1}),
        json!({"template_history": ["$(Unknown)"], "destination": 0, "append": false}),
        json!({"template_history": [""], "destination": 0, "append": false}),
        json!({"template_history": ["x", "x"], "destination": 0, "append": false}),
        json!({"template_history": [], "destination": 2, "path": "relative.txt", "append": false}),
        json!({"template_history": [], "destination": 2, "path": "\0", "append": false}),
    ] {
        let mut invalid = valid.clone();
        invalid["file_list"] = value;
        assert!(decode_preferences(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("file_list");
    assert!(decode_preferences(&serde_json::to_vec(&missing).unwrap()).is_err());
    let mut premature = valid;
    premature["version"] = 25.into();
    assert!(decode_preferences(&serde_json::to_vec(&premature).unwrap()).is_err());
}

#[test]
fn file_list_legacy_defaults_keep_minimal_versions_and_new_schema_preserves_required_fields() {
    let oldest =
        decode_preferences(br#"{"version":1,"theme":"blue","density":"desktop"}"#).unwrap();
    assert_eq!(
        oldest.file_list,
        midnight_core::FileListPreferences::default()
    );
    for size in [72, 96] {
        let preferences = UserPreferences {
            thumbnail_size: size,
            ..Default::default()
        };
        let bytes = encode_preferences(preferences.clone()).unwrap();
        let document: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["version"], if size == 96 { 10 } else { 25 });
        assert!(document.get("file_list").is_none());
        assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
    }
    let mut preferences = UserPreferences::default();
    preferences
        .file_list
        .remember("$(FileName:max) $(FileSize)$(LF)")
        .unwrap();
    let bytes = encode_preferences(preferences.clone()).unwrap();
    let mut document: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(document["version"], 26);
    assert_eq!(document["thumbnail_size"], 96);
    assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
    document.as_object_mut().unwrap().remove("thumbnail_size");
    assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
}

#[test]
fn file_list_storage_rejects_excessive_history_and_roundtrips_long_paths() {
    let mut preferences = UserPreferences::default();
    preferences.file_list.path = Some(
        std::env::temp_dir()
            .join("資料".repeat(400))
            .join("list.txt"),
    );
    preferences.file_list.destination = 2;
    let bytes = encode_preferences(preferences.clone()).unwrap();
    assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
    let valid: Value = serde_json::from_slice(&bytes).unwrap();
    for history in [
        json!((0..31).map(|index| index.to_string()).collect::<Vec<_>>()),
        json!(["x".repeat(midnight_core::MAX_FILE_LIST_TEMPLATE_BYTES + 1)]),
    ] {
        let mut invalid = valid.clone();
        invalid["file_list"]["template_history"] = history;
        assert!(decode_preferences(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    preferences.file_list.template_history = vec!["duplicate".into(), "duplicate".into()];
    assert!(encode_preferences(preferences).is_err());
}
