use super::*;

fn start_scan(p: &mut Presenter, pane: PaneId) -> FolderSizeScanId {
    let commands = RefCell::new(Vec::new());
    p.folder_sizes(pane, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    match commands.into_inner().pop().unwrap() {
        Command::ComputeFolderSizes { scan, .. } => scan,
        _ => panic!("expected a single scan request"),
    }
}

fn update(
    p: &mut Presenter,
    navigation: NavigationId,
    scan: FolderSizeScanId,
    entry: u64,
    bytes: u64,
) {
    p.apply(Event::FolderSizeUpdated {
        pane: PaneId::Left,
        navigation,
        scan,
        entry: EntryId(entry),
        size: FolderSize {
            bytes,
            finished: false,
            ..Default::default()
        },
    });
}

#[test]
fn cancelled_scan_accepts_its_final_partial_until_rerun_or_navigation() {
    let root = test_root().join("panel");
    let folder = root.join("folder");
    let mut p = Presenter::new(root.clone(), test_root());
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(7), folder.clone(), EntryKind::Directory)],
    );
    let old = start_scan(&mut p, PaneId::Left);
    p.apply(Event::FolderSizesFinished {
        pane: PaneId::Left,
        navigation,
        scan: old,
        cancelled: true,
    });
    assert!(
        p.panes[0].size_scan.is_none(),
        "cancelled scan is no longer shown as running"
    );
    update(&mut p, navigation, old, 7, 123);
    assert_eq!(p.panes[0].folder_sizes[&folder].bytes, 123);
    let new = start_scan(&mut p, PaneId::Left);
    update(&mut p, navigation, new, 7, 456);
    update(&mut p, navigation, old, 7, 999);
    assert_eq!(p.panes[0].folder_sizes[&folder].bytes, 456);
    p.navigate(PaneId::Left, test_root().join("elsewhere"), None, &|_| true);
    update(&mut p, navigation, new, 7, 999);
    assert!(p.panes[0].folder_sizes.is_empty());
}

#[test]
fn active_scan_continues_through_selection_and_repeated_cached_sorts() {
    let root = test_root().join("panel");
    let folder = root.join("folder");
    let rows = vec![Entry::new(EntryId(7), folder.clone(), EntryKind::Directory)];
    let mut p = Presenter::new(root.clone(), test_root());
    let original = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(&mut p, PaneId::Left, original, rows.clone());
    let scan = start_scan(&mut p, PaneId::Left);
    update(&mut p, original, scan, 7, 10);
    select_row(&mut p, PaneId::Left, 1, true);
    move_cursor(&mut p, PaneId::Left, 3);
    let sent = RefCell::new(Vec::new());
    for column in [SortColumn::Size, SortColumn::Name, SortColumn::Size] {
        assert!(p.sort(PaneId::Left, column, &|command| {
            sent.borrow_mut().push(command);
            true
        }));
        assert_eq!(p.panes[0].size_scan, Some(scan));
        update(&mut p, original, scan, 7, 20);
        assert_eq!(p.panes[0].folder_sizes[&folder].bytes, 20);
        let current = NavigationId(p.panes[0].generation);
        finish(&mut p, PaneId::Left, current, rows.clone());
        update(&mut p, original, scan, 7, 30);
        assert_eq!(p.panes[0].folder_sizes[&folder].bytes, 30);
    }
    assert!(
        sent.borrow()
            .iter()
            .all(|command| matches!(command, Command::Resort { .. }))
    );
    p.apply(Event::FolderSizesFinished {
        pane: PaneId::Left,
        navigation: original,
        scan,
        cancelled: false,
    });
    assert_eq!(p.panes[0].size_scan, None);
    assert_eq!(p.panes[0].folder_sizes[&folder].bytes, 30);
}

#[test]
fn same_path_refresh_preserves_scan_and_maps_recycled_ids_by_original_raw_path() {
    let root = test_root().join("panel");
    let folder = root.join("original");
    let unrelated = root.join("new");
    let mut p = Presenter::new(root.clone(), test_root());
    let original = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        original,
        vec![Entry::new(EntryId(7), folder.clone(), EntryKind::Directory)],
    );
    let scan = start_scan(&mut p, PaneId::Left);
    assert!(p.navigate(PaneId::Left, root.clone(), None, &|command| {
        assert!(matches!(command, Command::Refresh { scan: Some(actual), .. } if actual == scan));
        true
    }));
    update(&mut p, original, scan, 7, 10);
    assert_eq!(p.panes[0].folder_sizes[&folder].bytes, 10);
    let refreshed = NavigationId(p.panes[0].generation);
    finish(
        &mut p,
        PaneId::Left,
        refreshed,
        vec![
            Entry::new(EntryId(7), unrelated.clone(), EntryKind::Directory),
            Entry::new(EntryId(99), folder.clone(), EntryKind::Directory),
        ],
    );
    update(&mut p, original, scan, 7, 42);
    assert_eq!(p.panes[0].folder_sizes[&folder].bytes, 42);
    assert!(!p.panes[0].folder_sizes.contains_key(&unrelated));
    assert_eq!(p.panes[0].size_scan, Some(scan));
    assert!(p.navigate(PaneId::Left, test_root().join("other"), None, &|_| true));
    update(&mut p, original, scan, 7, 99);
    assert!(p.panes[0].folder_sizes.is_empty());
    assert!(p.panes[0].size_scan_scope.is_none());
}

#[test]
fn rejected_sort_and_refresh_leave_active_scan_and_cached_progress_unchanged() {
    let root = test_root().join("panel");
    let folder = root.join("folder");
    let mut p = Presenter::new(root.clone(), test_root());
    let original = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        original,
        vec![Entry::new(EntryId(7), folder.clone(), EntryKind::Directory)],
    );
    let scan = start_scan(&mut p, PaneId::Left);
    update(&mut p, original, scan, 7, 55);
    let previous_sort = p.panes[0].sort;
    assert!(!p.sort(PaneId::Left, SortColumn::Size, &|_| false));
    assert!(!p.navigate(PaneId::Left, root, None, &|_| false));
    assert_eq!(p.panes[0].size_scan, Some(scan));
    assert_eq!(p.panes[0].generation, original.0);
    assert_eq!(p.panes[0].sort, previous_sort);
    assert_eq!(p.panes[0].folder_sizes[&folder].bytes, 55);
}

#[test]
fn progress_and_completion_never_resort_the_displayed_listing() {
    let root = test_root().join("panel");
    let mut p = Presenter::new(root.clone(), test_root());
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(7), root.join("a"), EntryKind::Directory),
            Entry::new(EntryId(8), root.join("b"), EntryKind::Directory),
        ],
    );
    p.panes[0].sort.column = SortColumn::Size;
    let displayed = p.panes[0].entries.clone();
    let scan = start_scan(&mut p, PaneId::Left);
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    for finished in [false, true] {
        for (entry, bytes) in [(EntryId(7), 100), (EntryId(8), 1)] {
            p.apply_with_sender(
                Event::FolderSizeUpdated {
                    pane: PaneId::Left,
                    navigation,
                    scan,
                    entry,
                    size: FolderSize {
                        bytes,
                        finished,
                        ..Default::default()
                    },
                },
                &send,
            );
            assert!(Arc::ptr_eq(&displayed, &p.panes[0].entries));
        }
    }
    p.apply_with_sender(
        Event::FolderSizesFinished {
            pane: PaneId::Left,
            navigation,
            scan,
            cancelled: false,
        },
        &send,
    );
    assert!(Arc::ptr_eq(&displayed, &p.panes[0].entries));
    assert!(commands.borrow().is_empty());
    assert_eq!(p.panes[0].folder_sizes[&root.join("a")].bytes, 100);
    assert_eq!(p.panes[0].folder_sizes[&root.join("b")].bytes, 1);
}
