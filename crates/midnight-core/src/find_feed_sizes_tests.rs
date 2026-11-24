use super::*;

fn request(selected: Vec<bool>) -> FindFeedSizeRequest {
    let root = std::env::temp_dir();
    FindFeedSizeRequest {
        pane: PaneId::Left,
        id: FolderSizeScanId(1),
        rows_serial: 7,
        entries: Arc::from([
            Entry::new(EntryId(1), root.join("a/文档"), EntryKind::Directory),
            Entry::new(EntryId(2), root.join("b/file.txt"), EntryKind::File),
            Entry::new(EntryId(3), root.join("c/folder"), EntryKind::Directory),
        ]),
        selected: Arc::new(selected),
        cancel: Cancellation::default(),
    }
}

#[test]
fn unmarked_feed_measures_all_shown_folders_and_marked_files_never_fall_back() {
    let all = request(vec![false; 3]);
    assert_eq!(
        all.roots()
            .unwrap()
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        [EntryId(1), EntryId(3)]
    );
    assert!(
        request(vec![false, true, false])
            .roots()
            .unwrap()
            .is_empty()
    );
    let marked = request(vec![true, true, false]);
    let roots = marked.roots().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].path, std::env::temp_dir().join("a/文档"));
}

#[test]
fn capture_rejects_misaligned_or_reused_ids_and_keeps_raw_roots_frozen() {
    assert!(request(vec![false]).roots().is_err());
    let mut captured = request(vec![false; 3]);
    let frozen = captured.clone();
    captured.entries = Arc::from([]);
    assert_eq!(frozen.roots().unwrap().len(), 2);
    captured = request(vec![false; 3]);
    let mut entries = captured.entries.to_vec();
    entries[1].id = EntryId(1);
    captured.entries = entries.into();
    assert!(captured.roots().is_err());
}
