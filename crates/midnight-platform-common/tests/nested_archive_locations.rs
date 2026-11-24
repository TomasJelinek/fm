//! Schema 42: a Hot Path or saved tab inside archives inside archives keeps every level,
//! by raw member names, through restart and Export/Import, up to the deepest path that
//! opens (an archive opened from a native file and three archives inside it).
use midnight_core::*;
use midnight_platform_common::settings::{
    decode_preferences, decode_settings_file, encode_preferences, encode_settings_file,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn folder() -> PathBuf {
    std::env::current_dir().unwrap().join("data")
}

fn level(member: &str, format: ArchiveFormat, folders: &[&str]) -> NestedArchiveAddress {
    NestedArchiveAddress {
        member: member.as_bytes().to_vec(),
        format,
        components: folders
            .iter()
            .map(|name| name.as_bytes().to_vec())
            .collect(),
    }
}

/// `data\a.zip\inner\b.7z\deeper\c.tar.gz\folder\d.zip\last`.
fn deepest(folder: &Path) -> ArchiveAddress {
    ArchiveAddress {
        provider: "midnight.fs.archive".into(),
        format: Some(ArchiveFormat::Zip),
        container: folder.join("a.zip"),
        source_version: vec![3, 1, 4],
        components: vec![b"inner".to_vec()],
        nested: vec![
            level("b.7z", ArchiveFormat::SevenZip, &["deeper"]),
            level("c.tar.gz", ArchiveFormat::TarGzip, &["folder"]),
            level("d.zip", ArchiveFormat::Zip, &["last"]),
        ],
    }
}

fn preferences() -> UserPreferences {
    let folder = folder();
    let mut tabs = PaneTabPreferences {
        paths: vec![folder.clone(), folder.clone()],
        active: 1,
        archives: BTreeMap::new(),
        networks: BTreeMap::new(),
    };
    let mut shallower = deepest(&folder);
    shallower.truncate_steps(2);
    tabs.archives.insert(0, shallower);
    tabs.archives.insert(1, deepest(&folder));
    UserPreferences {
        bookmarks: vec![Bookmark {
            id: BookmarkId(7),
            name: "deep".into(),
            path: folder.clone(),
            archive: Some(deepest(&folder)),
            network: None,
            slot: Some(4),
        }],
        tabs: [tabs, PaneTabPreferences::default()],
        ..Default::default()
    }
}

#[test]
fn hot_paths_and_tabs_four_archives_deep_survive_restart_and_export() {
    let saved = preferences();
    let bytes = encode_preferences(saved.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["version"], 42);
    let stored = &json["bookmarks"][0]["archive"];
    assert_eq!(stored["version"], 3);
    assert_eq!(stored["nested"].as_array().unwrap().len(), 3);
    assert_eq!(stored["nested"][1]["format"], ArchiveFormat::TarGzip.id());
    assert_eq!(decode_preferences(&bytes).unwrap(), saved);
    // Export and Import write and read the same document.
    let exported = encode_settings_file(saved.clone()).unwrap();
    assert_eq!(decode_settings_file(&exported).unwrap().preferences, saved);
    // The Hot Paths list shows the whole path as the path bar does.
    let separator = std::path::MAIN_SEPARATOR;
    let shown = saved.bookmarks[0].display_text();
    assert!(
        shown.ends_with(&format!(
            "a.zip{separator}inner{separator}b.7z{separator}deeper{separator}c.tar.gz\
             {separator}folder{separator}d.zip{separator}last"
        )),
        "{shown}"
    );
}

#[test]
fn a_member_stays_a_member_and_a_folder_stays_a_folder() {
    let folder = folder();
    let nested = deepest(&folder);
    // The same names as plain folders of the outermost archive.
    let mut plain = nested.clone();
    plain.components = nested
        .steps()
        .into_iter()
        .map(|step| {
            let (ArchiveStep::Folder(name) | ArchiveStep::Archive(name, _)) = step;
            name.to_vec()
        })
        .collect();
    plain.nested.clear();
    let bookmark = |id, slot, archive| Bookmark {
        id: BookmarkId(id),
        name: "x".into(),
        path: folder.clone(),
        archive: Some(archive),
        network: None,
        slot: Some(slot),
    };
    let saved = UserPreferences {
        bookmarks: vec![bookmark(1, 1, nested), bookmark(2, 2, plain)],
        ..Default::default()
    };
    // Two Hot Paths, not one: equal display text names different locations.
    let decoded = decode_preferences(&encode_preferences(saved.clone()).unwrap()).unwrap();
    assert_eq!(decoded, saved);
    assert!(!decoded.bookmarks[0].same_location(&decoded.bookmarks[1]));
    assert_eq!(
        decoded.bookmarks[0].display_text(),
        decoded.bookmarks[1].display_text()
    );
}

#[test]
fn nested_locations_need_schema_42_and_schema_42_needs_them() {
    // Every field of schema 41 (Find) is present, so only the nested location is too new.
    let nested = UserPreferences {
        find: schema_41_find(),
        ..preferences()
    };
    let mut json: serde_json::Value =
        serde_json::from_slice(&encode_preferences(nested).unwrap()).unwrap();
    json["version"] = 41.into();
    let error = decode_preferences(&serde_json::to_vec(&json).unwrap()).unwrap_err();
    assert!(error.to_string().contains("version 42"), "{error}");
    // A schema 42 file without such a location was not written by this version.
    let schema_41 = UserPreferences {
        find: schema_41_find(),
        ..Default::default()
    };
    let mut json: serde_json::Value =
        serde_json::from_slice(&encode_preferences(schema_41).unwrap()).unwrap();
    assert_eq!(json["version"], 41);
    json["version"] = 42.into();
    let error = decode_preferences(&serde_json::to_vec(&json).unwrap()).unwrap_err();
    assert!(error.to_string().contains("schema 42"), "{error}");
}

/// Find preferences that need schema 41 of their own.
fn schema_41_find() -> FindPreferences {
    FindPreferences {
        window_geometry: None,
        mask_history: vec!["*.txt".into()],
    }
}

#[test]
fn damaged_nested_locations_are_refused_whole() {
    let mut json: serde_json::Value =
        serde_json::from_slice(&encode_preferences(preferences()).unwrap()).unwrap();
    let mut too_deep = json.clone();
    let levels = too_deep["bookmarks"][0]["archive"]["nested"].clone();
    too_deep["bookmarks"][0]["archive"]["nested"]
        .as_array_mut()
        .unwrap()
        .push(levels[0].clone());
    assert!(decode_preferences(&serde_json::to_vec(&too_deep).unwrap()).is_err());
    let mut unknown = json.clone();
    unknown["bookmarks"][0]["archive"]["nested"][0]["format"] = "rar5".into();
    assert!(decode_preferences(&serde_json::to_vec(&unknown).unwrap()).is_err());
    let mut parent = json.clone();
    parent["bookmarks"][0]["archive"]["nested"][2]["member"] = serde_json::json!(b"..");
    assert!(decode_preferences(&serde_json::to_vec(&parent).unwrap()).is_err());
    // Nested levels belong to address version 3 only.
    json["bookmarks"][0]["archive"]["version"] = 2.into();
    assert!(decode_preferences(&serde_json::to_vec(&json).unwrap()).is_err());
}
