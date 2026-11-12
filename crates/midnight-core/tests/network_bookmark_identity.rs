use midnight_core::*;
use std::sync::Arc;

fn saved() -> Bookmark {
    Bookmark {
        id: BookmarkId(1),
        name: "Network target".into(),
        path: std::env::current_dir().unwrap(),
        archive: None,
        network: Some(NetworkAddress {
            provider: "network.test".into(),
            opaque: Arc::from([0, 255, 3, 128]),
            display: "Caption is not a locator".into(),
        }),
        slot: Some(1),
    }
}

#[test]
fn discovery_hot_path_identity_ignores_caption_and_native_return_path() {
    let original = saved();
    let mut renamed = original.clone();
    renamed.name = "Renamed".into();
    renamed.path.push("different return folder");
    renamed.network.as_mut().unwrap().display = "Another caption".into();
    assert!(original.same_location(&renamed));
    assert_eq!(original.display_text(), "Caption is not a locator");
    let mut native = original.clone();
    native.network = None;
    assert!(!original.same_location(&native));
    let mut other_provider = original.clone();
    other_provider.network.as_mut().unwrap().provider = "network.other".into();
    assert!(!original.same_location(&other_provider));
    let mut other_address = original.clone();
    other_address.network.as_mut().unwrap().opaque = Arc::from([0, 255, 4, 128]);
    assert!(!original.same_location(&other_address));
}

#[test]
fn network_hot_path_rejects_ambiguous_or_invalid_typed_locations() {
    let mut bookmark = saved();
    bookmark.archive = Some(ArchiveAddress {
        provider: "midnight.fs.archive".into(),
        format: None,
        container: bookmark.path.join("archive.zip"),
        source_version: vec![1],
        components: vec![],
    });
    assert!(bookmark.validate().is_err());
    let mut bookmark = saved();
    bookmark.network.as_mut().unwrap().provider = "../wrong".into();
    assert!(bookmark.validate().is_err());
    let mut bookmark = saved();
    bookmark.network.as_mut().unwrap().opaque = Arc::default();
    assert!(bookmark.validate().is_err());
}

#[test]
fn network_hot_path_duplicates_merge_but_shortcut_slots_remain_unique() {
    let original = saved();
    let mut duplicate = original.clone();
    duplicate.id = BookmarkId(2);
    duplicate.slot = None;
    duplicate.network.as_mut().unwrap().display = "New caption".into();
    let mut preferences = UserPreferences {
        bookmarks: vec![original.clone(), duplicate],
        ..Default::default()
    };
    assert!(preferences.validate().is_err());
    assert!(preferences.deduplicate_bookmarks());
    assert_eq!(
        preferences.bookmarks.as_slice(),
        std::slice::from_ref(&original)
    );
    preferences.validate().unwrap();
    let mut distinct = original;
    distinct.id = BookmarkId(3);
    distinct.network.as_mut().unwrap().opaque = Arc::from([9]);
    preferences.bookmarks.push(distinct);
    assert!(
        preferences.validate().is_err(),
        "two locations cannot own one slot"
    );
    preferences.bookmarks[1].slot = Some(2);
    preferences.validate().unwrap();
}
