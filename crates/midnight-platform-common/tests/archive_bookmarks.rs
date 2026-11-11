//! Native/archive Hot Paths preserve raw member identity across application sessions.
use midnight_core::*;
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

fn bookmark(format: Option<ArchiveFormat>) -> Bookmark {
    let parent = std::env::current_dir().unwrap();
    Bookmark {
        network: None,
        id: BookmarkId(1),
        name: "folder".into(),
        path: parent.clone(),
        archive: Some(ArchiveAddress {
            provider: "midnight.fs.archive".into(),
            container: parent.join("日本語@data.zip"),
            format,
            source_version: vec![1, 0, 255],
            components: vec![b"nested@folder".to_vec(), vec![0xff, b'x']],
        }),
        slot: Some(10),
    }
}

#[test]
fn archive_hot_paths_round_trip_without_parsing_the_displayed_address() {
    for (format, version) in [(None, 19), (Some(ArchiveFormat::Zip), 20)] {
        let preferences = UserPreferences {
            bookmarks: vec![bookmark(format)],
            ..Default::default()
        };
        let bytes = encode_preferences(preferences.clone()).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["version"], version);
        assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
        assert_eq!(
            preferences.bookmarks[0]
                .archive
                .as_ref()
                .unwrap()
                .components[1],
            [0xff, b'x']
        );
        assert!(
            preferences.bookmarks[0]
                .display_text()
                .contains("日本語@data.zip@nested@folder")
        );
    }
}

#[test]
fn bookmark_locations_distinguish_archive_members_from_native_return_path() {
    let saved = bookmark(None);
    let mut newer = saved.clone();
    newer.archive.as_mut().unwrap().source_version = vec![2];
    assert!(saved.same_location(&newer));
    let mut other_member = saved.clone();
    other_member
        .archive
        .as_mut()
        .unwrap()
        .components
        .push(b"deeper".to_vec());
    assert!(!saved.same_location(&other_member));
    let mut native_parent = saved.clone();
    native_parent.archive = None;
    assert!(!saved.same_location(&native_parent));
    other_member.id = BookmarkId(2);
    other_member.slot = Some(2);
    native_parent.id = BookmarkId(3);
    native_parent.slot = Some(3);
    let preferences = UserPreferences {
        bookmarks: vec![saved, other_member, native_parent],
        ..Default::default()
    };
    assert_eq!(
        decode_preferences(&encode_preferences(preferences.clone()).unwrap()).unwrap(),
        preferences
    );
}

#[test]
fn archive_bookmarks_cannot_be_smuggled_into_older_settings_versions() {
    for (format, older_version) in [(None, 18), (Some(ArchiveFormat::Zip), 19)] {
        let preferences = UserPreferences {
            bookmarks: vec![bookmark(format)],
            ..Default::default()
        };
        let mut json: serde_json::Value =
            serde_json::from_slice(&encode_preferences(preferences).unwrap()).unwrap();
        json["version"] = older_version.into();
        assert!(decode_preferences(&serde_json::to_vec(&json).unwrap()).is_err());
    }
}

#[test]
fn archive_bookmarks_reject_invalid_members_and_unrelated_return_paths() {
    let mut saved = bookmark(None);
    saved.archive.as_mut().unwrap().components = vec![b"..".to_vec()];
    assert!(saved.validate().is_err());
    let mut saved = bookmark(None);
    saved.path.push("unrelated");
    assert!(saved.validate().is_err());
}

#[test]
fn decoding_does_not_discard_a_distinct_archive_decoder_location() {
    let zip = bookmark(Some(ArchiveFormat::Zip));
    let mut tar = bookmark(Some(ArchiveFormat::Tar));
    tar.id = BookmarkId(2);
    tar.slot = Some(2);
    let expected = UserPreferences {
        bookmarks: vec![zip, tar],
        ..Default::default()
    };
    assert_eq!(
        decode_preferences(&encode_preferences(expected.clone()).unwrap()).unwrap(),
        expected
    );
}
