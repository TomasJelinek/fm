use midnight_core::{
    ARCHIVE_FORMATS, ArchiveAddress, ArchiveFormat, Bookmark, BookmarkId, UserPreferences,
};
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

fn custom_preferences() -> UserPreferences {
    let mut preferences = UserPreferences::default();
    for (index, format) in ARCHIVE_FORMATS.iter().copied().enumerate() {
        preferences
            .archive_custom_extensions
            .insert(format!("自定{index}.backup"), format);
    }
    preferences
        .archive_association_overrides
        .insert("docx".into(), true);
    preferences
}

#[test]
fn all_custom_decoders_and_existing_checkboxes_survive_restart() {
    let expected = custom_preferences();
    let bytes = encode_preferences(expected.clone()).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(document["version"], 20);
    assert_eq!(document["archive_custom_extensions"]["自定0.backup"], "zip");
    assert_eq!(document["archive_association_overrides"]["docx"], true);
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);
}

#[test]
fn malformed_duplicate_conflicting_and_legacy_custom_maps_fail_atomically() {
    let document: serde_json::Value =
        serde_json::from_slice(&encode_preferences(custom_preferences()).unwrap()).unwrap();
    for value in [
        serde_json::json!(null),
        serde_json::json!([]),
        serde_json::json!({"zip":"tar"}),
        serde_json::json!({"DOCX":"zip"}),
        serde_json::json!({"custom":"unknown"}),
        serde_json::json!({"custom":true}),
        serde_json::json!({".custom":"zip"}),
        serde_json::json!({"*.custom":"zip"}),
        serde_json::json!({"Straße":"zip"}),
    ] {
        let mut bad = document.clone();
        bad["archive_custom_extensions"] = value;
        assert!(decode_preferences(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    let mut duplicate = document.clone();
    duplicate["archive_custom_extensions"] = serde_json::json!({"custom":"zip"});
    let duplicate = serde_json::to_string(&duplicate).unwrap().replace(
        "\"custom\":\"zip\"",
        "\"custom\":\"zip\",\"custom\":\"tar\"",
    );
    assert!(decode_preferences(duplicate.as_bytes()).is_err());
    let mut bad = document.clone();
    bad["version"] = 19.into();
    assert!(decode_preferences(&serde_json::to_vec(&bad).unwrap()).is_err());
    let mut bad = document;
    bad.as_object_mut()
        .unwrap()
        .remove("archive_custom_extensions");
    assert!(
        decode_preferences(&serde_json::to_vec(&bad).unwrap())
            .unwrap()
            .archive_custom_extensions
            .is_empty()
    );
}

#[test]
fn typed_decoder_persists_without_custom_rule_in_saved_tab_and_bookmark() {
    let root = std::env::current_dir().unwrap();
    let address = ArchiveAddress {
        provider: "midnight.fs.archive".into(),
        format: Some(ArchiveFormat::TarXz),
        container: root.join("文档.custom"),
        source_version: vec![1],
        components: vec![b"folder".to_vec()],
    };
    let mut expected = UserPreferences::default();
    expected.tabs[0].paths = vec![root];
    expected.tabs[0].archives.insert(0, address.clone());
    expected.bookmarks.push(Bookmark {
        network: None,
        id: BookmarkId(1),
        name: "Archive".into(),
        path: address.container.parent().unwrap().to_owned(),
        archive: Some(address),
        slot: None,
    });
    let bytes = encode_preferences(expected.clone()).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(document["version"], 20);
    assert_eq!(document["tabs"][0]["archives"]["0"]["version"], 2);
    assert_eq!(document["bookmarks"][0]["archive"]["format"], "tar-xz");
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);
    for value in [serde_json::json!("unknown"), serde_json::json!(null)] {
        let mut bad = document.clone();
        bad["bookmarks"][0]["archive"]["format"] = value;
        assert!(decode_preferences(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
}
