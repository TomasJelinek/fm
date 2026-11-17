use super::*;
#[test]
fn captured_archive_hot_path_keeps_raw_address_after_navigation_and_stays_unique() {
    let root = std::env::temp_dir().join("hotpath-capture");
    let mut p = Presenter::new(root.clone(), root.clone());
    p.preferences.loaded = true;
    let address = ArchiveAddress {
        provider: "archive".into(),
        format: Some(ArchiveFormat::Zip),
        container: root.join("captured.zip"),
        source_version: vec![1, 2, 3],
        components: vec!["文档".as_bytes().to_vec()],
    };
    p.panes[0].archive = Some(archives::ArchivePane::unavailable(
        address.clone(),
        root.clone(),
    ));
    let captured = p
        .capture_bookmark_location(PaneId::Left, "Saved archive".into())
        .unwrap();
    p.panes[0].archive = None;
    p.panes[0].path = root.join("later-native-folder");
    assert!(p.add_bookmark_location(captured.clone(), &|_| true));
    assert!(p.add_bookmark_location(captured, &|_| true));
    assert_eq!(p.bookmarks().len(), 1);
    assert_eq!(p.bookmarks()[0].archive, Some(address));
    assert_eq!(p.bookmarks()[0].path, root);
    assert_eq!(p.bookmarks()[0].slot, Some(1));
}
// The numbered-slot swap of captured locations (add_bookmark captures through
// capture_bookmark_location/add_bookmark_location) is covered by
// tests::bookmarks::moving_an_existing_hot_path_to_an_occupied_slot_keeps_both_paths.
