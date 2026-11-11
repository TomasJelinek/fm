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
#[test]
fn captured_hot_paths_keep_existing_numbered_slot_swap_rules() {
    let root = std::env::temp_dir().join("hotpath-slots");
    let mut p = Presenter::new(root.join("one"), root.join("two"));
    p.preferences.loaded = true;
    let first = p
        .capture_bookmark_location(PaneId::Left, "One".into())
        .unwrap();
    let second = p
        .capture_bookmark_location(PaneId::Right, "Two".into())
        .unwrap();
    assert!(p.add_bookmark_location(first, &|_| true));
    assert!(p.add_bookmark_location(second, &|_| true));
    let id = p.bookmarks()[0].id;
    assert!(p.set_bookmark_slot(id, Some(2), &|_| true));
    assert_eq!(p.bookmarks()[0].slot, Some(2));
    assert_eq!(p.bookmarks()[1].slot, Some(1));
    assert_eq!(p.bookmarks()[0].path, root.join("one"));
    assert_eq!(p.bookmarks()[1].path, root.join("two"));
}
