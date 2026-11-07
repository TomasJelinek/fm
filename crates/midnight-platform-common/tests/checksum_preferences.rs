use midnight_core::{ChecksumAlgorithm, ChecksumAlgorithms, UserPreferences};
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

#[test]
fn checksum_choice_roundtrips_and_legacy_preferences_keep_sha256() {
    let legacy = br#"{"version":1,"theme":"blue","density":"desktop"}"#;
    assert_eq!(
        decode_preferences(legacy).unwrap().checksum_algorithms,
        ChecksumAlgorithms::default()
    );
    let preferences = UserPreferences {
        checksum_algorithms: ChecksumAlgorithms::new(&[
            ChecksumAlgorithm::Md5,
            ChecksumAlgorithm::Sha512,
        ])
        .unwrap(),
        ..UserPreferences::default()
    };
    let stored = encode_preferences(preferences.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&stored).unwrap();
    assert_eq!(value["version"], 16);
    assert_eq!(decode_preferences(&stored).unwrap(), preferences);
    assert_eq!(
        encode_preferences(decode_preferences(&stored).unwrap()).unwrap(),
        stored
    );
}

#[test]
fn checksum_choices_reject_empty_unknown_duplicate_and_cross_version_fields() {
    let preferences = UserPreferences {
        checksum_algorithms: ChecksumAlgorithms::all(),
        ..UserPreferences::default()
    };
    let original: serde_json::Value =
        serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
    for invalid in [
        serde_json::json!([]),
        serde_json::json!(["sha256", "sha256"]),
        serde_json::json!(["crc32"]),
        serde_json::json!(null),
        serde_json::json!([1]),
    ] {
        let mut stored = original.clone();
        stored["checksum_algorithms"] = invalid;
        assert!(decode_preferences(&serde_json::to_vec(&stored).unwrap()).is_err());
    }
    let mut stored = original.clone();
    stored
        .as_object_mut()
        .unwrap()
        .remove("checksum_algorithms");
    assert!(decode_preferences(&serde_json::to_vec(&stored).unwrap()).is_err());
    let mut stored = original;
    stored["version"] = serde_json::json!(15);
    assert!(decode_preferences(&serde_json::to_vec(&stored).unwrap()).is_err());
}
