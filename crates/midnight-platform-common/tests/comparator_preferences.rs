use midnight_core::UserPreferences;
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

fn stored() -> serde_json::Value {
    let mut preferences = UserPreferences::default();
    preferences.viewer.wrap = true;
    serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap()
}

#[test]
fn comparator_font_survives_restart_and_keeps_other_viewer_choices() {
    let mut json = stored();
    json["version"] = 22.into();
    json["viewer"]["comparison_font"] = "Cascadia Mono".into();
    let decoded = decode_preferences(&serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(decoded.viewer.wrap);
    let saved: serde_json::Value =
        serde_json::from_slice(&encode_preferences(decoded).unwrap()).unwrap();
    assert_eq!(saved["viewer"]["comparison_font"], "Cascadia Mono");
    assert_eq!(saved["version"], 22);
}

#[test]
fn comparator_font_rejects_downgrade_controls_empty_and_oversized_names() {
    for name in [String::new(), "bad\nfont".into(), "x".repeat(257)] {
        let mut json = stored();
        json["version"] = 22.into();
        json["viewer"]["comparison_font"] = name.into();
        assert!(decode_preferences(&serde_json::to_vec(&json).unwrap()).is_err());
    }
    let mut json = stored();
    json["viewer"]["comparison_font"] = "Consolas".into();
    assert!(decode_preferences(&serde_json::to_vec(&json).unwrap()).is_err());
}

#[test]
fn legacy_settings_default_to_consolas_without_changing_existing_fonts() {
    let json = stored();
    assert_eq!(json["version"], 21);
    let preferences = decode_preferences(&serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(preferences.viewer.comparison_font, "Consolas");
    assert_eq!(preferences.viewer.text_font.family, "monospace");
    assert_eq!(preferences.viewer.source_font.family, "monospace");
}
