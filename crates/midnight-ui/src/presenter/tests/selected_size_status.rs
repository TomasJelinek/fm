use super::*;

fn selected_panel() -> Presenter {
    let root = test_root().join("size-status");
    let mut p = Presenter::new(root.clone(), root.clone());
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    let mut entries = vec![
        Entry::new(EntryId(1), root.join("file"), EntryKind::File),
        Entry::new(EntryId(2), root.join("known"), EntryKind::Directory),
        Entry::new(EntryId(3), root.join("partial"), EntryKind::Directory),
        Entry::new(EntryId(4), root.join("link"), EntryKind::DirectoryLink),
    ];
    entries[0].metadata.size = Some(123);
    finish(&mut p, PaneId::Left, navigation, entries);
    p.panes[0].selected =
        Arc::new(SelectionSnapshot::from_bits(p.panes[0].index.clone(), vec![15]).unwrap());
    p
}

fn request(p: &mut Presenter) -> SelectionSizeRequest {
    let requests = RefCell::new(Vec::new());
    p.flush_selection_sizes(&|command| {
        requests.borrow_mut().push(command);
        true
    });
    match requests.into_inner().pop().unwrap() {
        Command::SummarizeSelectedFolders(request) => request,
        _ => panic!("expected captured folder-total summary"),
    }
}

fn result(request: &SelectionSizeRequest, version: u64, bytes: u64) -> Event {
    Event::SelectedFolderSizes {
        request: request.id,
        pane: request.pane,
        navigation: request.navigation,
        selection_revision: request.revision,
        version,
        summary: SelectedFolderSizes {
            folders: 3,
            complete: 1,
            complete_bytes: Some(bytes),
            partial: 1,
            partial_bytes: Some(8),
        },
    }
}

#[test]
fn selected_status_keeps_file_bytes_separate_and_labels_partial_unknown_folders() {
    let mut p = selected_panel();
    let captured = request(&mut p);
    let fields = p.panel_details(PaneId::Left);
    assert_eq!(
        fields.len(),
        3,
        "mixed selections need separate file and folder byte fields"
    );
    assert!(fields[1].value.contains("123"));
    assert!(fields[2].value.contains("updating"));
    p.apply(result(&captured, 1, 42));
    let fields = p.panel_details(PaneId::Left);
    assert!(fields[2].value.contains("42 bytes"));
    assert!(fields[2].value.contains("1 of 3"));
    assert!(fields[2].value.contains("≈ 8 bytes"));
    assert!(fields[2].value.contains("1 partial"));
    assert!(fields[2].value.contains("1 unknown"));
}

#[test]
fn stale_totals_and_pending_selection_never_reuse_another_selections_exact_bytes() {
    let mut p = selected_panel();
    let captured = request(&mut p);
    p.apply(result(&captured, 2, 42));
    p.apply(result(&captured, 1, 999));
    assert!(p.panel_details(PaneId::Left)[2].value.contains("42 bytes"));
    let token = p.panel_details(PaneId::Left)[2].identity.clone();
    p.panes[0].selection_revision = SelectionRevision(1);
    p.panes[0].selection_pending_bulk = true;
    assert!(p.panel_details(PaneId::Left)[2].value.contains("updating"));
    assert!(!p.copy_panel_detail(PaneId::Left, &token, &|_| panic!("stale summary copied")));
    p.panes[0].selection_pending_bulk = false;
    p.panes[0].selection_applied = SelectionRevision(1);
    let next = request(&mut p);
    p.apply(result(&captured, 3, 777));
    assert!(p.panel_details(PaneId::Left)[2].value.contains("updating"));
    p.apply(result(&next, 4, 19));
    assert!(p.panel_details(PaneId::Left)[2].value.contains("19 bytes"));
}

#[test]
fn selection_summary_queue_backpressure_can_be_retried_without_recapturing_paths() {
    let mut p = selected_panel();
    p.flush_selection_sizes(&|_| false);
    let captured = request(&mut p);
    assert!(Arc::ptr_eq(&captured.selection, &p.panes[0].selected));
    p.flush_selection_sizes(&|_| panic!("same selection must not be requested per render"));
}

#[test]
fn selecting_in_a_new_tab_never_reuses_an_old_summary_request_identity() {
    let mut p = selected_panel();
    let first = request(&mut p);
    let rows = p.panes[0].entries.to_vec();
    assert!(p.new_tab(PaneId::Left, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    finish(&mut p, PaneId::Left, navigation, rows);
    p.panes[0].selected =
        Arc::new(SelectionSnapshot::from_bits(p.panes[0].index.clone(), vec![15]).unwrap());
    let next = request(&mut p);
    assert!(
        next.id > first.id,
        "backend latest-request guards must accept the new tab"
    );
    p.apply(result(&first, 99, 999));
    assert!(p.panel_details(PaneId::Left)[2].value.contains("updating"));
    p.apply(result(&next, 100, 23));
    assert!(p.panel_details(PaneId::Left)[2].value.contains("23 bytes"));
}

#[test]
fn failed_navigation_captures_the_restored_displayed_cache_generation() {
    let mut p = selected_panel();
    let source = p.panes[0].displayed_generation();
    let missing = test_root().join("missing-size-status");
    assert!(p.navigate(PaneId::Left, missing.clone(), None, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation,
        error: FsError::from_io("read directory", &missing, io::ErrorKind::NotFound.into()),
    });
    let captured = request(&mut p);
    assert_ne!(navigation, source);
    assert_eq!(captured.navigation, navigation);
    assert_eq!(captured.source_navigation, source);
}

#[test]
fn archive_status_uses_cached_provider_totals_and_never_requests_native_folder_sizes() {
    let mut p = selected_panel();
    let origin = p.panes[0].path.clone();
    p.panes[0].archive = Some(crate::presenter::archives::ArchivePane::unavailable(
        ArchiveAddress {
            provider: "test-provider".into(),
            container: origin.join("資料.zip"),
            source_version: vec![1],
            components: vec![],
        },
        origin,
    ));
    let mut totals = ArchiveSelectionTotals::default();
    totals.change(EntryKind::File, Some(777), true);
    totals.change(EntryKind::Directory, None, true);
    p.apply(Event::ArchiveSelection {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        selected: Arc::new(HashSet::from([EntryId(1), EntryId(2)])),
        totals,
    });
    let fields = p.panel_details(PaneId::Left);
    assert_eq!(fields[0].value, "1 file, 1 folder");
    assert!(fields[1].value.contains("777 bytes"));
    assert!(fields[2].value.contains("1 unknown"));
    p.flush_selection_sizes(&|_| panic!("virtual selection must not request native cache data"));
}
