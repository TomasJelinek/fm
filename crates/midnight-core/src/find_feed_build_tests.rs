use super::*;

fn result(id: u64, path: &str, kind: EntryKind, size: Option<u64>) -> Arc<FindResult> {
    Arc::new(FindResult {
        id: FindResultId(id),
        entry: crate::FindEntry {
            path: std::env::temp_dir().join(path),
            kind,
            metadata: crate::EntryMetadata {
                size,
                ..Default::default()
            },
            identity: crate::TransferIdentity {
                device: 1,
                object: [id as u8; 16],
            },
            revision: crate::FindRevision([0; 6]),
        },
        duplicate_group: None,
        duplicate_group_size: 0,
        archive: None,
    })
}
fn input() -> FindFeedBuildRequest {
    FindFeedBuildRequest {
        id: 1,
        pane: crate::PaneId::Left,
        session: crate::FindSessionId(1),
        generation: crate::FindGeneration(1),
        results: vec![
            result(1, "feed-build/a/one.txt", EntryKind::File, Some(10)),
            result(2, "feed-build/b", EntryKind::Directory, None),
            result(3, "feed-build/a/one.txt", EntryKind::File, Some(10)),
            result(4, "feed-build/c/文档.log", EntryKind::File, Some(20)),
        ]
        .into(),
        order: None,
        selection_bits: vec![0b1100].into(),
        explicit_marks: true,
        cursor: Some(FindResultId(3)),
        cancel: Cancellation::default(),
    }
}
#[test]
fn prepare_preserves_display_order_raw_paths_duplicate_marks_cursor_and_backend_index() {
    let mut request = input();
    request.order = Some(Arc::new(FindOrder {
        revision: 1,
        sort: crate::FindSortSpec::default(),
        ids: vec![
            FindResultId(4),
            FindResultId(2),
            FindResultId(1),
            FindResultId(3),
        ]
        .into(),
        positions: vec![2, 1, 3, 0].into(),
    }));
    let built = request.prepare().unwrap();
    assert_eq!(built.entries.len(), 3);
    assert_eq!(
        built
            .entries
            .iter()
            .map(|e| e.path.clone())
            .collect::<Vec<_>>(),
        [
            request.results[3].entry.path.clone(),
            request.results[1].entry.path.clone(),
            request.results[0].entry.path.clone()
        ]
    );
    assert_eq!(*built.selected, [true, false, true]);
    assert_eq!(
        (
            built.selected_count,
            built.selected_bytes,
            built.cursor,
            built.omitted
        ),
        (2, 30, 2, 0)
    );
    assert_eq!(
        built.index.by_path[&request.results[0].entry.path],
        EntryId(3)
    );
    assert_eq!(built.index.by_id[&EntryId(1)], 0);
}
#[test]
fn cursor_only_selection_starts_unmarked_and_nonregular_rows_are_omitted() {
    let mut request = input();
    let mut rows = request.results.to_vec();
    rows.push(result(5, "feed-build/link", EntryKind::Symlink, None));
    request.results = rows.into();
    request.selection_bits = vec![1].into();
    request.explicit_marks = false;
    request.cursor = Some(FindResultId(1));
    let built = request.prepare().unwrap();
    assert_eq!(built.omitted, 1);
    assert_eq!(built.selected_count, 0);
    assert!(built.selected.iter().all(|selected| !*selected));
    assert_eq!(built.cursor, 0);
}
#[test]
fn cancelled_oversized_bad_order_and_missing_native_results_fail_without_partial_output() {
    let mut request = input();
    request.cancel.cancel();
    assert_eq!(
        request.prepare().unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    request.cancel = Cancellation::default();
    request.selection_bits = vec![0; FindLimits::default().max_results.div_ceil(64) + 1].into();
    assert!(request.prepare().is_err());
    request = input();
    request.order = Some(Arc::new(FindOrder {
        revision: 1,
        sort: crate::FindSortSpec::default(),
        ids: vec![FindResultId(1); 4].into(),
        positions: vec![0; 4].into(),
    }));
    assert!(request.prepare().is_err());
    request.results = vec![result(1, "feed-build/link", EntryKind::Symlink, None)].into();
    request.order = None;
    request.selection_bits = Arc::from([]);
    assert!(request.prepare().is_err());
}

#[test]
fn a_sorted_prefix_keeps_its_order_before_the_streamed_discovery_tail() {
    let mut request = input();
    request.results = vec![
        result(1, "prefix/one", EntryKind::File, None),
        result(2, "prefix/two", EntryKind::File, None),
        result(3, "prefix/three", EntryKind::File, None),
    ]
    .into();
    request.order = Some(Arc::new(FindOrder {
        revision: 1,
        sort: crate::FindSortSpec::default(),
        ids: vec![FindResultId(2), FindResultId(1)].into(),
        positions: vec![1, 0].into(),
    }));
    request.selection_bits = Arc::from([]);
    request.explicit_marks = false;
    let built = request.prepare().unwrap();
    assert_eq!(
        built
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>(),
        [
            request.results[1].entry.path.clone(),
            request.results[0].entry.path.clone(),
            request.results[2].entry.path.clone()
        ]
    );
    request.order = Some(Arc::new(FindOrder {
        revision: 1,
        sort: crate::FindSortSpec::default(),
        ids: vec![FindResultId(3), FindResultId(1)].into(),
        positions: vec![1, 0].into(),
    }));
    assert!(
        request.prepare().is_err(),
        "a prefix cannot capture an ID from its streamed tail"
    );
}
