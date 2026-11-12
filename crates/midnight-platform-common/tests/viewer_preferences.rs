//! Persist viewer options through the complete strict application settings codec.
use midnight_core::*;
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

#[test]
fn viewer_choices_survive_restart_without_losing_other_options() {
    let mut expected = UserPreferences {
        theme: PreferenceTheme::Dark,
        disabled_viewer_plugins: vec!["midnight.viewer.source".into()],
        ..Default::default()
    };
    for change in [
        ViewerPreferenceChange::Wrap(true),
        ViewerPreferenceChange::LineNumbers(false),
        ViewerPreferenceChange::CaseSensitive(true),
        ViewerPreferenceChange::FontFamily(ViewerFontKind::Text, "Noto Sans Mono 日本語".into()),
        ViewerPreferenceChange::FontSize(ViewerFontKind::Text, 17),
        ViewerPreferenceChange::FontSize(ViewerFontKind::Source, 18),
        ViewerPreferenceChange::FontSize(ViewerFontKind::Markdown, 20),
        ViewerPreferenceChange::MarkdownPreset(MarkdownPreset::Comfortable),
        ViewerPreferenceChange::ImageZoom(ViewerZoomPreference::ActualPixels),
        ViewerPreferenceChange::DocumentZoom(ViewerZoomPreference::ScaleBasisPoints(12500)),
    ] {
        expected.viewer.apply(&change).unwrap();
    }
    let bytes = encode_preferences(expected.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["version"], 21);
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("search_query") && !text.contains("document_contents"));
}

#[test]
fn legacy_settings_get_viewer_defaults_and_new_values_cannot_be_downgraded() {
    let old = encode_preferences(UserPreferences::default()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&old).unwrap();
    assert_eq!(json["version"], 10);
    assert!(json.get("viewer").is_none());
    assert_eq!(
        decode_preferences(&old).unwrap().viewer,
        ViewerPreferences::default()
    );

    let mut value = UserPreferences::default();
    value.viewer.wrap = true;
    let mut json: serde_json::Value =
        serde_json::from_slice(&encode_preferences(value).unwrap()).unwrap();
    json["version"] = 20.into();
    assert!(decode_preferences(&serde_json::to_vec(&json).unwrap()).is_err());
    json["version"] = 21.into();
    json.as_object_mut().unwrap().remove("viewer");
    assert!(decode_preferences(&serde_json::to_vec(&json).unwrap()).is_err());
}

#[test]
fn malformed_viewer_settings_fail_atomically() {
    let mut value = UserPreferences::default();
    value.viewer.wrap = true;
    let base: serde_json::Value =
        serde_json::from_slice(&encode_preferences(value).unwrap()).unwrap();
    for (field, invalid) in [
        ("text_font", serde_json::json!({"family":"Mono","size":0})),
        (
            "image_zoom",
            serde_json::json!({"mode":"scale","basis_points":0}),
        ),
        ("markdown_preset", serde_json::json!("unknown")),
        ("search_query", serde_json::json!("must not be stored")),
    ] {
        let mut json = base.clone();
        json["viewer"][field] = invalid;
        assert!(
            decode_preferences(&serde_json::to_vec(&json).unwrap()).is_err(),
            "{field}"
        );
    }
}
