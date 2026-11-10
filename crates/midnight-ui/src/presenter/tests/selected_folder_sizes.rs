use super::*;

fn ready(bits: u64) -> Presenter {
    let root = test_root().join("selected-sizes");
    let mut p = Presenter::new(root.clone(), root.clone());
    let nav = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        nav,
        vec![
            Entry::new(EntryId(1), root.join("selected-資料"), EntryKind::Directory),
            Entry::new(EntryId(2), root.join("unselected"), EntryKind::Directory),
            Entry::new(EntryId(3), root.join("ordinary.txt"), EntryKind::File),
            Entry::new(EntryId(4), root.join("link"), EntryKind::DirectoryLink),
        ],
    );
    p.panes[0].selected =
        Arc::new(SelectionSnapshot::from_bits(p.panes[0].index.clone(), vec![bits]).unwrap());
    p
}

#[test]
fn selected_size_scan_keeps_unselected_cached_values_and_running_state() {
    let mut p = ready(1);
    let selected = p.panes[0].path.join("selected-資料");
    let unselected = p.panes[0].path.join("unselected");
    p.panes[0].folder_sizes.insert(
        unselected.clone(),
        FolderSize {
            bytes: 345,
            finished: true,
            ..Default::default()
        },
    );
    p.folder_sizes(PaneId::Left, &|_| true);
    assert_eq!(
        p.panes[0]
            .folder_sizes
            .get(&unselected)
            .map(|size| size.bytes),
        Some(345)
    );
    assert!(p.panes[0].entry_size_running(&selected));
    assert!(!p.panes[0].entry_size_running(&unselected));
    assert!(!p.panes[0].entry_size_running(&p.panes[0].path.join("link")));
}

#[test]
fn size_scan_waits_for_the_exact_pending_selection_revision() {
    let mut p = ready(0);
    p.panes[0].selection_revision = SelectionRevision(8);
    p.panes[0].selection_resolved = SelectionRevision(7);
    p.folder_sizes(PaneId::Left, &|_| {
        panic!("pending selection must not scan every directory")
    });
    assert!(p.panes[0].size_scan.is_none());
    let selected =
        Arc::new(SelectionSnapshot::from_bits(p.panes[0].index.clone(), vec![1]).unwrap());
    let navigation = NavigationId(p.panes[0].generation);
    let commands = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::SelectionChanged {
            pane: PaneId::Left,
            navigation,
            revision: SelectionRevision(8),
            selection: selected,
            deferred: Default::default(),
        },
        &|command| {
            commands.borrow_mut().push(command);
            true
        },
    );
    assert!(
        commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::ComputeFolderSizes { .. }))
    );
    assert!(!p.panes[0].entry_size_running(&p.panes[0].path.join("unselected")));
}

#[test]
fn selecting_only_files_never_marks_unselected_directories_as_running() {
    let mut p = ready(4);
    p.folder_sizes(PaneId::Left, &|_| true);
    assert!(!p.panes[0].entry_size_running(&p.panes[0].path.join("selected-資料")));
    assert!(!p.panes[0].entry_size_running(&p.panes[0].path.join("unselected")));
}

#[test]
fn captured_selection_survives_new_selection_and_sort_without_retargeting_results() {
    let mut p = ready(1);
    let original = NavigationId(p.panes[0].generation);
    let selected = p.panes[0].path.join("selected-資料");
    let unselected = p.panes[0].path.join("unselected");
    let captured = p.panes[0].selected.clone();
    let sent = RefCell::new(Vec::new());
    p.folder_sizes(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let scan = match sent.borrow().last().unwrap() {
        Command::ComputeFolderSizes {
            scan,
            selection: Some(actual),
            ..
        } => {
            assert!(Arc::ptr_eq(actual, &captured));
            *scan
        }
        _ => panic!("expected captured selected-directory scan"),
    };
    p.panes[0].selected =
        Arc::new(SelectionSnapshot::from_bits(p.panes[0].index.clone(), vec![2]).unwrap());
    let entries = p.panes[0].entries.to_vec();
    assert!(p.sort(PaneId::Left, SortColumn::Size, &|command| {
        assert!(matches!(command, Command::Resort { .. }));
        true
    }));
    let sorted = NavigationId(p.panes[0].generation);
    finish(&mut p, PaneId::Left, sorted, entries);
    for (entry, bytes) in [(1, 73), (2, 999)] {
        p.apply(Event::FolderSizeUpdated {
            pane: PaneId::Left,
            navigation: original,
            scan,
            entry: EntryId(entry),
            size: FolderSize {
                bytes,
                finished: true,
                ..Default::default()
            },
        });
    }
    assert_eq!(p.panes[0].size_scan, Some(scan));
    assert_eq!(p.panes[0].folder_sizes[&selected].bytes, 73);
    assert!(!p.panes[0].folder_sizes.contains_key(&unselected));
    assert!(p.panes[0].entry_size_running(&selected));
    assert!(!p.panes[0].entry_size_running(&unselected));
}

#[test]
fn no_selection_is_explicitly_distinct_from_an_empty_captured_selection() {
    let mut p = ready(0);
    p.folder_sizes(PaneId::Left, &|command| {
        assert!(matches!(
            command,
            Command::ComputeFolderSizes {
                selection: None,
                ..
            }
        ));
        true
    });
    assert!(p.panes[0].entry_size_running(&p.panes[0].path.join("selected-資料")));
    assert!(p.panes[0].entry_size_running(&p.panes[0].path.join("unselected")));
}
