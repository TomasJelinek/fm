use super::*;

fn entry(id: u64, name: &str, kind: EntryKind, size: u64) -> Entry {
    let mut entry = Entry::new(
        EntryId(id),
        std::env::temp_dir().join("feed-projection").join(name),
        kind,
    );
    entry.metadata.size = Some(size);
    entry
}

fn request() -> FindFeedProjectionRequest {
    let entries: Arc<[Entry]> = vec![
        entry(8, "b.log", EntryKind::File, 20),
        entry(3, "folder", EntryKind::Directory, 0),
        entry(21, "a.txt", EntryKind::File, 10),
    ]
    .into();
    FindFeedProjectionRequest {
        id: 1,
        pane: PaneId::Left,
        rows_serial: 10,
        selection_revision: 2,
        entries: entries.clone(),
        visible: entries,
        selected: Arc::new(vec![true, false, false]),
        cursor: Some(EntryId(8)),
        options: FindFeedOptions {
            show_hidden: true,
            ..Default::default()
        },
        mask: None,
        removed: Arc::from([]),
        cancel: Cancellation::default(),
    }
}

#[test]
fn sorted_rows_keep_raw_ids_marks_cursor_and_real_index_paths() {
    let request = request();
    let view = project(&request, None).unwrap();
    assert_eq!(
        view.entries.iter().map(|e| e.id.0).collect::<Vec<_>>(),
        [21, 8, 3]
    );
    assert_eq!(*view.selected, [false, true, false]);
    assert_eq!(
        (view.selected_count, view.selected_bytes, view.cursor),
        (1, 20, 1)
    );
    assert_eq!(view.index.by_path[&request.entries[0].path], EntryId(8));
}

#[test]
fn filter_restore_retains_originals_and_masks_only_current_visible_rows() {
    let mut request = request();
    request.options.filter = PanelFilter {
        enabled: true,
        mode: FilterMode::Expression,
        expression: "kind = file and size >= 15".into(),
        case_sensitive: false,
    };
    let first = project(&request, None).unwrap();
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.entries[0].id, EntryId(8));
    request.entries = first.all_entries;
    request.visible = first.entries;
    request.selected = Arc::new(vec![false]);
    request.options.filter.enabled = false;
    request.mask = Some(Arc::new(MaskSelection {
        pattern: "*.*".into(),
        select: true,
        folders: true,
    }));
    let restored = project(&request, None).unwrap();
    assert_eq!(restored.entries.len(), 3);
    let marked: Vec<_> = restored
        .entries
        .iter()
        .zip(restored.selected.iter())
        .filter_map(|(e, m)| m.then_some(e.id))
        .collect();
    assert_eq!(
        marked,
        [EntryId(8)],
        "mask cannot reach previously hidden rows"
    );
}

#[test]
fn removal_prunes_filtered_original_descendants_before_restore() {
    let mut request = request();
    let mut all = request.entries.to_vec();
    all.push(entry(31, "folder/hidden.txt", EntryKind::File, 8));
    request.entries = all.into();
    request.removed = vec![Arc::from(vec![request.entries[1].path.clone()])].into();
    let view = project(&request, None).unwrap();
    assert_eq!(view.all_entries.len(), 2);
    assert!(
        view.all_entries
            .iter()
            .all(|e| !e.path.starts_with(&request.entries[1].path))
    );
}

#[test]
fn invalid_filter_cancelled_request_and_mismatched_selection_fail_without_projection() {
    let mut request = request();
    request.options.filter = PanelFilter {
        enabled: true,
        mode: FilterMode::Regex,
        expression: "[".into(),
        case_sensitive: false,
    };
    assert!(project(&request, None).is_err());
    request.options.filter.enabled = false;
    request.cancel.cancel();
    assert_eq!(
        project(&request, None).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    request.cancel = Cancellation::default();
    request.selected = Arc::new(vec![]);
    assert_eq!(
        project(&request, None).unwrap_err().kind,
        io::ErrorKind::InvalidInput
    );
}

#[tokio::test]
async fn coalesced_slots_publish_latest_and_do_not_pin_finished_feed_inputs() {
    let (sender, mut events) = tokio::sync::mpsc::channel(1);
    sender.send(Event::VolumesUpdated(vec![])).await.unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    let controller = crate::filters::Controller::new(
        crate::Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let first = request();
    let cancel = first.cancel.clone();
    controller.project_feed(Arc::new(first), None);
    tokio::task::yield_now().await;
    let mut second = request();
    second.id = 2;
    let retained = Arc::downgrade(&second.entries);
    controller.project_feed(Arc::new(second), None);
    assert!(matches!(
        events.recv().await,
        Some(Event::VolumesUpdated(_))
    ));
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    let Event::FindFeedProjected(receipt) = event else {
        panic!("latest feed projection");
    };
    assert_eq!(receipt.request.id, 2);
    assert!(receipt.result.is_ok());
    assert!(cancel.is_cancelled());
    drop(receipt);
    assert!(
        retained.upgrade().is_none(),
        "idle watch slots must not pin an old feed"
    );
    drop(controller);
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
}

#[test]
fn hidden_projection_can_restore_originals_and_enforces_existing_locale_key_budget() {
    let mut request = request();
    let mut entries = request.entries.to_vec();
    let mut hidden = entry(55, ".hidden.txt", EntryKind::File, 1);
    hidden.metadata.native_attributes = Some(0x2);
    entries.push(hidden);
    request.entries = entries.into();
    request.options.show_hidden = false;
    let visible = project(&request, None).unwrap();
    assert!(!visible.entries.iter().any(|e| e.id == EntryId(55)));
    request.entries = visible.all_entries;
    request.options.show_hidden = true;
    let restored = project(&request, None).unwrap();
    assert!(restored.entries.iter().any(|e| e.id == EntryId(55)));
    struct LargeKeys;
    impl LocaleService for LargeKeys {
        fn number_locale(&self) -> Result<NumberLocale, FsError> {
            Ok(NumberLocale::default())
        }
        fn collation_key(&self, _: &std::ffi::OsStr) -> Result<Option<Vec<u8>>, FsError> {
            Ok(Some(vec![0; MAX_COLLATION_KEY_BYTES + 1]))
        }
    }
    let error = project(&request, Some(&LargeKeys)).unwrap_err();
    assert!(error.message.contains("bounded sort budget"), "{error}");
}
