//! Text fitting is an optional setting: the default adds nothing to the file.
use midnight_core::{GlyphFitting, UserPreferences};
use midnight_platform_common::settings::{decode_preferences, encode_preferences};
use serde_json::Value;

fn preferences(glyph_fitting: GlyphFitting) -> UserPreferences {
    UserPreferences {
        glyph_fitting,
        ..UserPreferences::default()
    }
}

#[test]
fn the_default_fitting_is_not_written() {
    let bytes = encode_preferences(preferences(GlyphFitting::Fitted)).unwrap();
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(document.get("glyph_fitting").is_none());
    assert_eq!(decode_preferences(&bytes).unwrap().glyph_fitting, GlyphFitting::Fitted);
}

#[test]
fn every_choice_roundtrips_with_the_current_version() {
    for choice in [GlyphFitting::Snapped, GlyphFitting::Subpixel] {
        let expected = preferences(choice);
        let bytes = encode_preferences(expected.clone()).unwrap();
        let document: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["version"], 34);
        assert_eq!(document["glyph_fitting"], choice.name());
        assert_eq!(decode_preferences(&bytes).unwrap(), expected);
    }
}

#[test]
fn unknown_names_and_older_versions_are_rejected() {
    let bytes = encode_preferences(preferences(GlyphFitting::Snapped)).unwrap();
    let mut document: Value = serde_json::from_slice(&bytes).unwrap();
    document["glyph_fitting"] = "blurry".into();
    assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
    document["glyph_fitting"] = "snapped".into();
    document["version"] = 33.into();
    assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
}
