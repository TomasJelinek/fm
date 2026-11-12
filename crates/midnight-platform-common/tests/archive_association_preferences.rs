use midnight_core::UserPreferences;
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

fn current_document() -> serde_json::Value {
    let mut document: serde_json::Value =
        serde_json::from_slice(&encode_preferences(UserPreferences::default()).unwrap()).unwrap();
    document["version"] = 13.into();
    document["previews"] = serde_json::json!({"enabled":true,"network":false});
    document["archive_association_overrides"] = serde_json::json!({"zip":false,"docx":true});
    document
}

#[test]
fn enabled_documents_and_disabled_archives_survive_restart() {
    let original = current_document();
    let decoded = decode_preferences(&serde_json::to_vec(&original).unwrap()).unwrap();
    let encoded = encode_preferences(decoded.clone()).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(stored["version"], 13);
    assert_eq!(
        stored["archive_association_overrides"],
        original["archive_association_overrides"]
    );
    assert_eq!(decode_preferences(&encoded).unwrap(), decoded);
}

#[test]
fn association_schema_rejects_unknown_noncanonical_missing_and_duplicate_choices() {
    let original = current_document();
    for value in [
        serde_json::Value::Null,
        serde_json::json!([]),
        serde_json::json!({"unknown":true}),
        serde_json::json!({"DOCX":true}),
        serde_json::json!({".docx":true}),
        serde_json::json!({"docx":"true"}),
    ] {
        let mut changed = original.clone();
        changed["archive_association_overrides"] = value;
        assert!(decode_preferences(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
    let mut missing = original.clone();
    missing
        .as_object_mut()
        .unwrap()
        .remove("archive_association_overrides");
    assert!(decode_preferences(&serde_json::to_vec(&missing).unwrap()).is_err());
    let duplicate = serde_json::to_string(&original).unwrap().replace(
        "\"archive_association_overrides\":{\"docx\":true,\"zip\":false}",
        "\"archive_association_overrides\":{\"docx\":true,\"docx\":false}",
    );
    assert!(decode_preferences(duplicate.as_bytes()).is_err());
    let mut legacy = original;
    legacy["version"] = 12.into();
    assert!(decode_preferences(&serde_json::to_vec(&legacy).unwrap()).is_err());
}

#[test]
fn legacy_settings_keep_default_encoding_without_an_association_field() {
    let encoded = encode_preferences(UserPreferences::default()).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(stored["version"], 10);
    assert!(stored.get("archive_association_overrides").is_none());
    assert_eq!(
        decode_preferences(&encoded).unwrap(),
        UserPreferences::default()
    );
}

#[test]
fn every_known_choice_is_bounded_validated_and_independent() {
    let defaults = UserPreferences::default();
    for association in midnight_core::ARCHIVE_ASSOCIATIONS {
        assert_eq!(
            defaults.archive_association_enabled(association.extension),
            association.enabled_by_default
        );
        let mut preferences = defaults.clone();
        preferences.archive_association_overrides.insert(
            association.extension.into(),
            !association.enabled_by_default,
        );
        assert_eq!(
            preferences.archive_association_enabled(association.extension),
            !association.enabled_by_default
        );
        let encoded = encode_preferences(preferences.clone()).unwrap();
        assert_eq!(decode_preferences(&encoded).unwrap(), preferences);
    }
    for extension in ["unknown", "DOCX", ".zip", "../zip", ""] {
        let mut invalid = defaults.clone();
        invalid
            .archive_association_overrides
            .insert(extension.into(), true);
        assert!(!invalid.archive_association_enabled(extension));
        assert!(invalid.validate().is_err());
        assert!(encode_preferences(invalid).is_err());
    }
}

#[test]
fn association_settings_preserve_preview_choices() {
    for enabled in [false, true] {
        for network in [false, true] {
            let mut preferences = UserPreferences {
                thumbnails_enabled: enabled,
                thumbnails_network: network,
                ..Default::default()
            };
            preferences
                .archive_association_overrides
                .insert("jar".into(), true);
            let encoded = encode_preferences(preferences.clone()).unwrap();
            assert_eq!(decode_preferences(&encoded).unwrap(), preferences);
        }
    }
}
