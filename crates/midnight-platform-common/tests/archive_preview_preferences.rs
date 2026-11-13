use midnight_core::{DiskIoConcurrency, PanelFilter, UserPreferences};
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

fn schema32_document() -> Value {
    let mut document = schema31_document();
    document["version"] = 32.into();
    document["disk_io_concurrency"] = json!({"mode":"automatic"});
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
fn disabled_archive_previews_roundtrip_independently_in_schema32() {
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
                assert_eq!(document["version"], 32);
                assert_eq!(document["disk_io_concurrency"], json!({"mode":"automatic"}));
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
        assert_eq!(
            preferences.disk_io_concurrency,
            DiskIoConcurrency::Automatic
        );
        let saved: Value =
            serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
        assert_eq!(saved["version"], if archives { 30 } else { 32 });
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
    for version in [0, 33, u32::MAX] {
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

#[test]
fn schema31_requires_exactly_one_of_the_two_published_extensions() {
    let archive_branch = schema31_document();
    let archive = decode_preferences(&serde_json::to_vec(&archive_branch).unwrap()).unwrap();
    assert!(!archive.thumbnails_archives);
    assert_eq!(archive.disk_io_concurrency, DiskIoConcurrency::Automatic);

    let mut disk_branch = archive_branch.clone();
    disk_branch["previews"]
        .as_object_mut()
        .unwrap()
        .remove("archives");
    disk_branch["disk_io_concurrency"] = json!({"mode":"manual","limit":7});
    let disk = decode_preferences(&serde_json::to_vec(&disk_branch).unwrap()).unwrap();
    assert!(disk.thumbnails_archives);
    assert_eq!(disk.disk_io_concurrency, DiskIoConcurrency::Manual(7));

    let mut neither = archive_branch.clone();
    neither["previews"]
        .as_object_mut()
        .unwrap()
        .remove("archives");
    assert!(decode_preferences(&serde_json::to_vec(&neither).unwrap()).is_err());
    for archives in [false, true] {
        for policy in [
            json!({"mode":"automatic"}),
            json!({"mode":"manual","limit":7}),
        ] {
            let mut both = archive_branch.clone();
            both["previews"]["archives"] = archives.into();
            both["disk_io_concurrency"] = policy;
            assert!(decode_preferences(&serde_json::to_vec(&both).unwrap()).is_err());
        }
    }
}

#[test]
fn schema31_cannot_treat_a_null_extension_as_the_other_published_branch() {
    let archive_branch = schema31_document();
    for malformed in [
        Value::Null,
        json!(true),
        json!(0),
        json!("automatic"),
        json!([]),
    ] {
        let mut document = archive_branch.clone();
        document["disk_io_concurrency"] = malformed;
        assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
    }
    let mut disk_branch = archive_branch;
    disk_branch["previews"]
        .as_object_mut()
        .unwrap()
        .remove("archives");
    disk_branch["disk_io_concurrency"] = json!({"mode":"manual","limit":7});
    for malformed in [Value::Null, json!(0), json!("false"), json!([]), json!({})] {
        let mut document = disk_branch.clone();
        document["previews"]["archives"] = malformed;
        assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
    }
}

#[test]
fn schema32_requires_both_extensions_and_preserves_their_independent_values() {
    for archives in [false, true] {
        for policy in [
            DiskIoConcurrency::Automatic,
            DiskIoConcurrency::Manual(1),
            DiskIoConcurrency::Manual(32),
        ] {
            let mut document = schema32_document();
            document["previews"]["archives"] = archives.into();
            document["disk_io_concurrency"] = match policy {
                DiskIoConcurrency::Automatic => json!({"mode":"automatic"}),
                DiskIoConcurrency::Manual(limit) => json!({"mode":"manual","limit":limit}),
            };
            let preferences = decode_preferences(&serde_json::to_vec(&document).unwrap()).unwrap();
            assert_eq!(preferences.thumbnails_archives, archives);
            assert_eq!(preferences.disk_io_concurrency, policy);
            assert_eq!(preferences.panel_filter_history.len(), 1);
            assert_eq!(
                decode_preferences(&encode_preferences(preferences.clone()).unwrap()).unwrap(),
                preferences,
            );
        }
    }
    let document = schema32_document();
    let mut missing_archives = document.clone();
    missing_archives["previews"]
        .as_object_mut()
        .unwrap()
        .remove("archives");
    assert!(decode_preferences(&serde_json::to_vec(&missing_archives).unwrap()).is_err());
    let mut missing_disk = document.clone();
    missing_disk
        .as_object_mut()
        .unwrap()
        .remove("disk_io_concurrency");
    assert!(decode_preferences(&serde_json::to_vec(&missing_disk).unwrap()).is_err());
    let mut neither = missing_disk;
    neither["previews"]
        .as_object_mut()
        .unwrap()
        .remove("archives");
    assert!(decode_preferences(&serde_json::to_vec(&neither).unwrap()).is_err());
    for field in ["disk_io_concurrency", "previews"] {
        let mut malformed = document.clone();
        malformed[field] = Value::Null;
        assert!(decode_preferences(&serde_json::to_vec(&malformed).unwrap()).is_err());
    }
    let mut malformed = document;
    malformed["previews"]["archives"] = Value::Null;
    assert!(decode_preferences(&serde_json::to_vec(&malformed).unwrap()).is_err());
}

#[test]
fn schema32_still_requires_previous_rules_history_and_preview_fields() {
    let original = schema32_document();
    assert!(decode_preferences(&serde_json::to_vec(&original).unwrap()).is_ok());
    for field in [
        "copy_checksum_verification",
        "file_list",
        "viewer_associations",
        "thumbnail_size",
        "editor",
        "panel_filter_history",
        "previews",
    ] {
        let mut document = original.clone();
        document.as_object_mut().unwrap().remove(field);
        assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
    }
    let mut missing_rules = original.clone();
    missing_rules["editor"]
        .as_object_mut()
        .unwrap()
        .remove("rules");
    assert!(decode_preferences(&serde_json::to_vec(&missing_rules).unwrap()).is_err());
    for field in ["enabled", "network"] {
        let mut document = original.clone();
        document["previews"].as_object_mut().unwrap().remove(field);
        assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
    }
}
