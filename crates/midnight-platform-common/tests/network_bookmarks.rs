//! Strict, lossless Network Hot Paths use the same opaque codec as typed tabs.
use midnight_core::*;
use midnight_platform_common::settings::{decode_preferences, encode_preferences};
use std::sync::Arc;

fn saved() -> Bookmark {
    Bookmark {
        id: BookmarkId(1),
        name: "Saved Network".into(),
        path: std::env::current_dir().unwrap(),
        archive: None,
        network: Some(NetworkAddress {
            provider: "network.absent-plugin".into(),
            opaque: Arc::from([0, 255, 128, b'@', b'/']),
            display: "資料 @ not-a-path".into(),
        }),
        slot: Some(10),
    }
}

#[test]
fn network_hot_paths_round_trip_without_plugin_or_caption_parsing() {
    for verification in [true, false] {
        let expected = UserPreferences {
            bookmarks: vec![saved()],
            copy_checksum_verification: verification,
            shortcuts: vec![ShortcutBinding {
                action: "bookmark.open.10".into(),
                keys: vec!["Ctrl+Shift+F12".into()],
            }],
            ..Default::default()
        };
        let bytes = encode_preferences(expected.clone()).unwrap();
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["version"], 29);
        assert_eq!(document["bookmarks"][0]["network"]["version"], 1);
        assert_eq!(
            document["bookmarks"][0]["network"]["opaque"],
            serde_json::json!([0, 255, 128, 64, 47])
        );
        assert_eq!(decode_preferences(&bytes).unwrap(), expected);
        let restarted = decode_preferences(&bytes).unwrap();
        let map = ShortcutMap::new(&restarted.shortcuts).unwrap();
        assert_eq!(
            map.resolve(
                &ShortcutChord::parse("Ctrl+Shift+F12").unwrap(),
                ShortcutContext::Files
            )
            .unwrap()
            .id,
            "bookmark.open.10"
        );
        assert!(
            map.resolve(
                &ShortcutChord::parse("Digit0").unwrap(),
                ShortcutContext::Files
            )
            .is_none()
        );
    }
}

#[test]
fn network_bookmark_schema_rejects_old_versions_unknown_fields_and_invalid_addresses() {
    let bytes = encode_preferences(UserPreferences {
        bookmarks: vec![saved()],
        ..Default::default()
    })
    .unwrap();
    let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for version in [24, 28] {
        let mut changed = document.clone();
        changed["version"] = version.into();
        assert!(decode_preferences(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
    for (field, value) in [
        ("version", serde_json::json!(2)),
        ("provider", serde_json::json!("../wrong")),
        ("opaque", serde_json::json!([])),
        (
            "credentials",
            serde_json::json!({"password": "must never be accepted"}),
        ),
    ] {
        let mut changed = document.clone();
        changed["bookmarks"][0]["network"][field] = value;
        assert!(
            decode_preferences(&serde_json::to_vec(&changed).unwrap()).is_err(),
            "{field}"
        );
    }
    let mut changed = document.clone();
    changed["bookmarks"][0]["network"] = serde_json::Value::Null;
    assert!(decode_preferences(&serde_json::to_vec(&changed).unwrap()).is_err());
}

#[test]
fn same_caption_distinct_network_identities_survive_and_duplicate_slots_are_rejected() {
    let first = saved();
    let mut second = first.clone();
    second.id = BookmarkId(2);
    second.slot = Some(1);
    second.network.as_mut().unwrap().opaque = Arc::from([1, 2, 3]);
    let preferences = UserPreferences {
        bookmarks: vec![first, second],
        ..Default::default()
    };
    let bytes = encode_preferences(preferences.clone()).unwrap();
    assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
    let mut document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    document["bookmarks"][1]["slot"] = document["bookmarks"][0]["slot"].clone();
    assert!(decode_preferences(&serde_json::to_vec(&document).unwrap()).is_err());
}
