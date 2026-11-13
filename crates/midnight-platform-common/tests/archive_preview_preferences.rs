use midnight_core::{PanelFilter, UserPreferences};
use midnight_platform_common::settings::{decode_preferences, encode_preferences};
use serde_json::{Value, json};

fn legacy_cases() -> Vec<(u32, UserPreferences)> {
    vec![
        (10, UserPreferences::default()),
        (
            12,
            UserPreferences {
                thumbnails_network: true,
                ..Default::default()
            },
        ),
        (
            25,
            UserPreferences {
                thumbnail_size: 192,
                ..Default::default()
            },
        ),
        (
            30,
            UserPreferences {
                panel_filter_history: vec![PanelFilter {
                    enabled: true,
                    expression: "*.png".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ),
    ]
}

fn schema31_document() -> Value {
    let (_, preferences) = legacy_cases().pop().unwrap();
    let mut document: Value =
        serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
    assert_eq!(document["version"], 30);
    document["version"] = 31.into();
    document["previews"]["archives"] = false.into();
    document
}

#[test]
fn archive_previews_default_true_and_preserve_legacy_encodings() {
    assert!(UserPreferences::default().thumbnails_archives);
    assert!(
        decode_preferences(br#"{"version":1,"theme":"blue","density":"desktop"}"#)
            .unwrap()
            .thumbnails_archives
    );
    for (version, preferences) in legacy_cases() {
        let bytes = encode_preferences(preferences.clone()).unwrap();
        let document: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["version"], version);
        assert!(document["previews"].get("archives").is_none());
        let decoded = decode_preferences(&bytes).unwrap();
        assert!(decoded.thumbnails_archives);
        assert_eq!(decoded, preferences);
    }
}

#[test]
fn disabled_archive_previews_roundtrip_independently_in_schema31() {
    for enabled in [false, true] {
        for network in [false, true] {
            for verification in [false, true] {
                let preferences = UserPreferences {
                    thumbnails_enabled: enabled,
                    thumbnails_network: network,
                    thumbnails_archives: false,
                    copy_checksum_verification: verification,
                    ..Default::default()
                };
                let bytes = encode_preferences(preferences.clone()).unwrap();
                let document: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(document["version"], 31);
                assert_eq!(
                    document["previews"],
                    json!({"enabled":enabled,"network":network,"archives":false}),
                );
                assert_eq!(document["editor"]["rules"], json!([]));
                assert_eq!(document["panel_filter_history"], json!([]));
                assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
            }
        }
    }
}

#[test]
fn schema31_accepts_explicit_archive_preview_booleans_and_omits_true_when_saved() {
    for archives in [false, true] {
        let mut document = schema31_document();
        document["previews"]["archives"] = archives.into();
        let preferences = decode_preferences(&serde_json::to_vec(&document).unwrap()).unwrap();
        assert_eq!(preferences.thumbnails_archives, archives);
        assert!(preferences.thumbnails_enabled);
        assert!(!preferences.thumbnails_network);
        assert_eq!(preferences.panel_filter_history.len(), 1);
        let saved: Value =
            serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
        assert_eq!(saved["version"], if archives { 30 } else { 31 });
        if archives {
            assert!(saved["previews"].get("archives").is_none());
        } else {
            assert_eq!(saved["previews"]["archives"], false);
        }
    }
}

#[test]
fn archive_preview_schema_rejects_missing_null_wrong_type_duplicate_and_premature_fields() {
    let document = schema31_document();
    for archives in [Value::Null, json!("false"), json!(0), json!([]), json!({})] {
        let mut changed = document.clone();
        changed["previews"]["archives"] = archives;
        assert!(decode_preferences(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
    let mut missing = document.clone();
    missing["previews"]
        .as_object_mut()
        .unwrap()
        .remove("archives");
    assert!(decode_preferences(&serde_json::to_vec(&missing).unwrap()).is_err());
    let mut missing = document.clone();
    missing.as_object_mut().unwrap().remove("previews");
    assert!(decode_preferences(&serde_json::to_vec(&missing).unwrap()).is_err());
    for (version, preferences) in legacy_cases() {
        let mut old: Value =
            serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
        assert_eq!(old["version"], version);
        if old.get("previews").is_none() {
            old["previews"] = json!({"enabled":true,"network":false});
        }
        for archives in [false, true] {
            old["previews"]["archives"] = archives.into();
            assert!(decode_preferences(&serde_json::to_vec(&old).unwrap()).is_err());
        }
    }
    let duplicate = serde_json::to_string(&document)
        .unwrap()
        .replace("\"archives\":false", "\"archives\":false,\"archives\":true");
    assert!(decode_preferences(duplicate.as_bytes()).is_err());
    for version in [0, 32, u32::MAX] {
        let mut invalid = document.clone();
        invalid["version"] = version.into();
        assert!(decode_preferences(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    let mut unknown = document.clone();
    unknown["previews"]["archive"] = true.into();
    assert!(decode_preferences(&serde_json::to_vec(&unknown).unwrap()).is_err());
    let mut unexpected = document;
    unexpected["thumbnails_archives"] = false.into();
    assert!(decode_preferences(&serde_json::to_vec(&unexpected).unwrap()).is_err());
}

#[test]
fn archive_preview_schema31_retains_existing_latest_schema_requirements() {
    for missing in [
        "copy_checksum_verification",
        "file_list",
        "viewer_associations",
        "user_commands",
        "checksum_algorithms",
        "panel_split_basis_points",
        "filename_colors",
        "archive_association_overrides",
        "disabled_viewer_plugins",
        "viewer",
        "editor",
        "font_family",
        "desktop_font_size",
        "touch_font_size",
        "quick_search_filter",
        "size_display",
        "remember_command_history",
        "command_history",
        "bookmarks",
        "tabs",
        "panel_view_modes",
        "shortcuts",
        "panel_filters",
        "previews",
        "thumbnail_size",
        "panel_filter_history",
    ] {
        let mut document = schema31_document();
        assert!(document.as_object_mut().unwrap().remove(missing).is_some());
        assert!(
            decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err(),
            "schema31 accepted missing {missing}",
        );
    }
    let mut document = schema31_document();
    document["editor"].as_object_mut().unwrap().remove("rules");
    assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
}
