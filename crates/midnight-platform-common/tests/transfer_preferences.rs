use midnight_core::UserPreferences;
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

#[test]
fn checksum_defaults_on_and_explicit_quick_mode_survives_restart() {
    let defaults = UserPreferences::default();
    assert!(defaults.copy_checksum_verification);
    let legacy = encode_preferences(defaults).unwrap();
    assert!(
        decode_preferences(&legacy)
            .unwrap()
            .copy_checksum_verification
    );
    let quick = UserPreferences {
        copy_checksum_verification: false,
        ..UserPreferences::default()
    };
    let bytes = encode_preferences(quick.clone()).unwrap();
    assert_eq!(decode_preferences(&bytes).unwrap(), quick);
    let mut saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(saved["version"], 28);
    saved["copy_checksum_verification"] = true.into();
    assert!(
        decode_preferences(&serde_json::to_vec(&saved).unwrap())
            .unwrap()
            .copy_checksum_verification
    );
    saved
        .as_object_mut()
        .unwrap()
        .remove("copy_checksum_verification");
    assert!(decode_preferences(&serde_json::to_vec(&saved).unwrap()).is_err());
    for invalid in [serde_json::Value::Null, "false".into(), 0.into()] {
        saved["copy_checksum_verification"] = invalid;
        assert!(decode_preferences(&serde_json::to_vec(&saved).unwrap()).is_err());
    }
}
