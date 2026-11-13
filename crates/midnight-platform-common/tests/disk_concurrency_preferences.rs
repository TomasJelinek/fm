//! Disk-I/O settings retain strict schema boundaries and the legacy automatic default.
use midnight_core::{DiskIoConcurrency, PreferenceTheme, UserPreferences};
use midnight_platform_common::settings::{decode_preferences, encode_preferences};
use serde_json::{Value, json};

fn manual_preferences(limit: u8) -> UserPreferences {
    UserPreferences {
        disk_io_concurrency: DiskIoConcurrency::Manual(limit),
        ..UserPreferences::default()
    }
}

fn manual_document() -> Value {
    let expected = manual_preferences(4);
    let bytes = encode_preferences(expected.clone()).unwrap();
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);
    serde_json::from_slice(&bytes).unwrap()
}

fn decode(document: &Value) -> std::io::Result<UserPreferences> {
    decode_preferences(&serde_json::to_vec(document).unwrap())
}

fn raw_disk_fields(fields: &str) -> String {
    let mut document = manual_document();
    document
        .as_object_mut()
        .unwrap()
        .remove("disk_io_concurrency");
    let mut prefix = serde_json::to_string(&document).unwrap();
    assert_eq!(prefix.pop(), Some('}'));
    format!("{prefix},{fields}}}")
}

#[test]
fn automatic_preserves_the_existing_minimum_schema_and_roundtrips() {
    let expected = UserPreferences::default();
    assert_eq!(expected.disk_io_concurrency, DiskIoConcurrency::Automatic);
    let bytes = encode_preferences(expected.clone()).unwrap();
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(document["version"], 10);
    assert!(document.get("disk_io_concurrency").is_none());
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);
}

#[test]
fn manual_boundaries_require_schema_31_and_roundtrip_all_preferences() {
    for limit in [1, 32] {
        let expected = manual_preferences(limit);
        let bytes = encode_preferences(expected.clone()).unwrap();
        let document: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["version"], 31);
        assert_eq!(
            document["disk_io_concurrency"],
            json!({"mode": "manual", "limit": limit})
        );
        assert_eq!(decode_preferences(&bytes).unwrap(), expected);
    }
}

#[test]
fn manual_choice_preserves_unrelated_nondefault_settings() {
    let mut expected = manual_preferences(7);
    expected.theme = PreferenceTheme::Dark;
    expected.copy_checksum_verification = false;
    expected.viewer.wrap = true;
    expected.font_family = "文档 é 🧪".into();
    expected.panel_split_basis_points = 3700;
    let bytes = encode_preferences(expected.clone()).unwrap();
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);
}

#[test]
fn explicit_automatic_in_schema_31_returns_to_minimum_schema_on_save() {
    let mut document = manual_document();
    document["disk_io_concurrency"] = json!({"mode": "automatic"});
    let preferences = decode(&document).unwrap();
    assert_eq!(preferences, UserPreferences::default());
    let saved: Value = serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
    assert_eq!(saved["version"], 10);
    assert!(saved.get("disk_io_concurrency").is_none());
}

#[test]
fn missing_legacy_values_migrate_to_automatic_without_losing_choices() {
    let legacy = br#"{"version":1,"theme":"dark","density":"touch","windows_shell":"cmd"}"#;
    let expected = decode_preferences(legacy).unwrap();
    assert_eq!(expected.disk_io_concurrency, DiskIoConcurrency::Automatic);
    assert_eq!(expected.theme, PreferenceTheme::Dark);
    assert_eq!(expected.density, midnight_core::UiDensity::Touch);
    assert_eq!(expected.windows_shell, midnight_core::WindowsShell::Cmd);
    let migrated = encode_preferences(expected.clone()).unwrap();
    assert_eq!(decode_preferences(&migrated).unwrap(), expected);

    let mut document = manual_document();
    document["version"] = 30.into();
    document
        .as_object_mut()
        .unwrap()
        .remove("disk_io_concurrency");
    let expected = decode(&document).unwrap();
    assert_eq!(expected, UserPreferences::default());
    assert_eq!(
        decode_preferences(&encode_preferences(expected.clone()).unwrap()).unwrap(),
        expected
    );
}

#[test]
fn invalid_manual_api_values_fail_before_serialization() {
    for limit in [0, 33, 255] {
        let error = encode_preferences(manual_preferences(limit)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}

#[test]
fn malformed_or_out_of_range_manual_values_are_rejected() {
    for limit in [
        json!(0),
        json!(33),
        json!(255),
        json!(256),
        json!(-1),
        json!(1.5),
        json!("4"),
        json!(null),
        json!(true),
    ] {
        let mut document = manual_document();
        document["disk_io_concurrency"] = json!({"mode": "manual", "limit": limit});
        assert!(decode(&document).is_err(), "accepted {document}");
    }
}

#[test]
fn unknown_missing_null_and_cross_mode_fields_are_rejected() {
    for value in [
        json!(null),
        json!(4),
        json!("automatic"),
        json!([]),
        json!({}),
        json!({"mode": null}),
        json!({"mode": "unknown"}),
        json!({"mode": "manual"}),
        json!({"limit": 4}),
        json!({"mode": "automatic", "limit": 4}),
        json!({"mode": "automatic", "limit": null}),
        json!({"mode": "automatic", "unknown": true}),
        json!({"mode": "manual", "limit": 4, "unknown": true}),
    ] {
        let mut document = manual_document();
        document["disk_io_concurrency"] = value;
        assert!(decode(&document).is_err(), "accepted {document}");
    }
    let mut document = manual_document();
    document["unexpected"] = true.into();
    assert!(decode(&document).is_err());
}

#[test]
fn duplicate_top_level_mode_and_limit_fields_are_rejected() {
    for fields in [
        r#""disk_io_concurrency":{"mode":"manual","limit":4},"disk_io_concurrency":{"mode":"automatic"}"#,
        r#""disk_io_concurrency":{"mode":"manual","mode":"automatic","limit":4}"#,
        r#""disk_io_concurrency":{"mode":"manual","limit":4,"limit":8}"#,
        r#""disk_io_concurrency":{"mode":"automatic","mode":"automatic"}"#,
    ] {
        let document = raw_disk_fields(fields);
        assert!(
            decode_preferences(document.as_bytes()).is_err(),
            "accepted {document}"
        );
    }
}

#[test]
fn disk_fields_cannot_cross_schema_boundaries_or_be_omitted_in_31() {
    for version in [0, 30, 32] {
        let mut document = manual_document();
        document["version"] = version.into();
        assert!(decode(&document).is_err());
    }
    for mode in [
        json!({"mode":"automatic"}),
        json!({"mode":"manual","limit":4}),
    ] {
        let mut document: Value =
            serde_json::from_slice(&encode_preferences(UserPreferences::default()).unwrap())
                .unwrap();
        document["disk_io_concurrency"] = mode;
        assert!(decode(&document).is_err());
    }
    let mut document = manual_document();
    document
        .as_object_mut()
        .unwrap()
        .remove("disk_io_concurrency");
    assert!(decode(&document).is_err());
}

#[test]
fn schema_31_still_requires_earlier_versioned_fields() {
    for field in [
        "file_list",
        "viewer_associations",
        "thumbnail_size",
        "user_commands",
        "checksum_algorithms",
        "panel_split_basis_points",
        "filename_colors",
        "archive_association_overrides",
        "previews",
        "viewer",
        "copy_checksum_verification",
        "panel_filter_history",
    ] {
        let mut document = manual_document();
        assert!(document.as_object_mut().unwrap().remove(field).is_some());
        assert!(decode(&document).is_err(), "accepted missing {field}");
    }
    let mut document = manual_document();
    assert!(
        document["editor"]
            .as_object_mut()
            .unwrap()
            .remove("rules")
            .is_some()
    );
    assert!(decode(&document).is_err());
}
