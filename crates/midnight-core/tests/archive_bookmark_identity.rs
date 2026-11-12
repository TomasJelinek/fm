use midnight_core::*;

fn bookmark(format: Option<ArchiveFormat>) -> Bookmark {
    let root = std::env::current_dir().unwrap();
    Bookmark {
        network: None,
        id: BookmarkId(1),
        name: "member".into(),
        path: root.clone(),
        slot: None,
        archive: Some(ArchiveAddress {
            provider: "midnight.fs.archive".into(),
            format,
            container: root.join("container.zip"),
            source_version: vec![1],
            components: vec![b"member".to_vec()],
        }),
    }
}

#[test]
fn distinct_explicit_decoders_are_not_merged_as_one_archive_location() {
    let zip = bookmark(Some(ArchiveFormat::Zip));
    let mut tar = bookmark(Some(ArchiveFormat::Tar));
    tar.id = BookmarkId(2);
    assert!(!zip.same_location(&tar));
    let mut prefs = UserPreferences {
        bookmarks: vec![zip, tar],
        ..Default::default()
    };
    assert!(!prefs.deduplicate_bookmarks());
    assert_eq!(prefs.bookmarks.len(), 2);
    prefs.validate().unwrap();
}

#[test]
fn legacy_inferred_decoder_matches_the_equivalent_explicit_decoder() {
    assert!(bookmark(None).same_location(&bookmark(Some(ArchiveFormat::Zip))));
    assert!(!bookmark(None).same_location(&bookmark(Some(ArchiveFormat::Tar))));
}
