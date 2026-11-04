use super::*;

#[test]
fn new_tab_duplicates_raw_location_and_rejected_switch_preserves_current_view() {
    let base = test_root().join("文档 e\u{301} 🧪");
    let mut p = Presenter::new(base.clone(), test_root());
    let first = begin(&mut p, PaneId::Left, base.to_str().unwrap());
    finish(&mut p, PaneId::Left, first, vec![entry(1, "retained")]);
    let original = p.tabs(PaneId::Left)[0].id;
    assert!(!p.new_tab(PaneId::Left, &|_| false));
    assert_eq!(p.tabs(PaneId::Left).len(), 1);
    assert_eq!(p.panes[0].entries.len(), 1);
    assert!(p.new_tab(PaneId::Left, &|command| {
        assert!(matches!(command, Command::Navigate { ref path, .. } if path == &base));
        true
    }));
    let tabs = p.tabs(PaneId::Left);
    assert_eq!(tabs.len(), 2);
    assert_ne!(tabs[1].id, original);
    assert_eq!(tabs[1].path, base);
    assert!(tabs[1].active);
    assert_eq!(p.tabs(PaneId::Right).len(), 1);
    assert!(!p.select_tab(PaneId::Left, original, &|_| false));
    assert!(p.tabs(PaneId::Left)[1].active);
}

#[test]
fn parked_tab_results_never_replace_same_path_tab_and_generations_never_rewind() {
    let base = test_root();
    let mut p = Presenter::new(base.clone(), base.clone());
    let first = begin(&mut p, PaneId::Left, base.to_str().unwrap());
    let original = p.tabs(PaneId::Left)[0].id;
    assert!(p.new_tab(PaneId::Left, &|_| true));
    let second = NavigationId(p.panes[0].generation);
    assert!(second.0 > first.0);
    finish(&mut p, PaneId::Left, first, vec![entry(1, "old tab")]);
    assert!(p.panes[0].entries.is_empty());
    finish(&mut p, PaneId::Left, second, vec![entry(2, "new tab")]);
    assert_eq!(p.panes[0].entries[0].id, EntryId(2));
    assert!(p.select_tab(PaneId::Left, original, &|_| true));
    let third = NavigationId(p.panes[0].generation);
    assert!(third.0 > second.0);
    p.apply(Event::MetadataBatch {
        pane: PaneId::Left,
        navigation: second,
        entries: vec![(EntryId(2), EntryMetadata::default())],
    });
    finish(&mut p, PaneId::Left, second, vec![entry(2, "late new tab")]);
    assert!(p.panes[0].entries.is_empty());
    assert!(p.panes[0].metadata.is_empty());
    finish(&mut p, PaneId::Left, third, vec![entry(3, "restored tab")]);
    assert_eq!(p.panes[0].entries[0].id, EntryId(3));
}

#[test]
fn tab_histories_and_sort_are_independent_and_last_tab_close_is_noop() {
    let base = test_root();
    let mut p = Presenter::new(base.clone(), base.clone());
    let original = p.tabs(PaneId::Left)[0].id;
    assert!(!p.close_active_tab(PaneId::Left, &|_| panic!("last tab must remain")));
    for path in [base.clone(), base.join("a")] {
        let id = begin(&mut p, PaneId::Left, path.to_str().unwrap());
        finish(&mut p, PaneId::Left, id, vec![]);
    }
    p.panes[0].sort = SortSpec {
        column: SortColumn::Size,
        descending: true,
    };
    assert!(p.new_tab(PaneId::Left, &|_| true));
    let new = p.tabs(PaneId::Left)[1].id;
    let generation = NavigationId(p.panes[0].generation);
    finish(&mut p, PaneId::Left, generation, vec![]);
    let next = begin(&mut p, PaneId::Left, base.join("b").to_str().unwrap());
    finish(&mut p, PaneId::Left, next, vec![]);
    p.panes[0].sort = SortSpec::default();
    assert!(p.select_tab(PaneId::Left, original, &|_| true));
    assert_eq!(p.panes[0].history, [base.clone(), base.join("a")]);
    assert_eq!(p.panes[0].sort.column, SortColumn::Size);
    assert!(!p.close_tab(PaneId::Left, original, &|_| false));
    assert_eq!(p.tabs(PaneId::Left).len(), 2);
    assert!(p.close_tab(PaneId::Left, original, &|_| true));
    assert_eq!(p.tabs(PaneId::Left)[0].id, new);
    assert_eq!(p.panes[0].display_path(), base.join("b"));
    assert!(!p.close_tab(PaneId::Left, new, &|_| panic!("keep last tab")));
}

#[test]
fn closing_a_tab_cannot_revive_a_delayed_directory_open_from_the_same_path() {
    let base = test_root().join("base");
    let link = base.join("link");
    let mut p = Presenter::new(base.clone(), test_root());
    let first = begin(&mut p, PaneId::Left, base.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        first,
        vec![Entry::new(EntryId(7), link.clone(), EntryKind::Symlink)],
    );
    let opened = RefCell::new(None);
    assert!(p.open(PaneId::Left, 1, &|command| {
        *opened.borrow_mut() = Some(command);
        true
    }));
    let Command::OpenEntry {
        request,
        navigation,
        ..
    } = opened.into_inner().unwrap()
    else {
        panic!("expected native link classification")
    };
    assert!(p.new_tab(PaneId::Left, &|_| true));
    let second = NavigationId(p.panes[0].generation);
    assert!(p.close_active_tab(PaneId::Left, &|_| true));
    assert!(p.panes[0].generation > second.0);
    p.apply_with_sender(
        Event::DesktopActionFinished {
            request,
            result: Ok(DesktopActionResult::OpenDirectory {
                pane: PaneId::Left,
                navigation,
                path: link,
            }),
        },
        &|_| panic!("closed generation must not navigate any tab"),
    );
    assert_eq!(p.panes[0].display_path(), base);
}

#[test]
fn parked_tabs_release_snapshots_on_the_backend_and_restore_cursor_by_raw_path() {
    let base = test_root().join("base");
    let cursor_path = base.join("文档 e\u{301} 🧪");
    let mut p = Presenter::new(base.clone(), test_root());
    let original = p.tabs(PaneId::Left)[0].id;
    let first = begin(&mut p, PaneId::Left, base.to_str().unwrap());
    let entries: Arc<[Entry]> =
        vec![Entry::new(EntryId(7), cursor_path.clone(), EntryKind::File)].into();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    let weak_entries = Arc::downgrade(&entries);
    let weak_index = Arc::downgrade(&index);
    p.apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: first,
        entries,
        index,
    });
    select_row(&mut p, PaneId::Left, 1, true);
    assert!(p.new_tab(PaneId::Left, &|_| true));
    assert!(weak_entries.upgrade().is_some());
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(weak_entries.upgrade().is_none());
    assert!(weak_index.upgrade().is_none());
    assert!(p.select_tab(PaneId::Left, original, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(99), cursor_path, EntryKind::File)],
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(99)));
    assert!(
        p.panes[0].selected.is_empty(),
        "reactivation refresh resets selection"
    );
}

#[test]
fn tab_cycling_wraps_within_the_chosen_pane_and_inactive_close_never_navigates() {
    let mut p = Presenter::new(test_root(), test_root());
    let original = p.tabs(PaneId::Left)[0].id;
    assert!(p.new_tab(PaneId::Left, &|_| true));
    let second = p.tabs(PaneId::Left)[1].id;
    assert!(p.cycle_tab(PaneId::Left, 1, &|_| true));
    assert!(p.tabs(PaneId::Left)[0].active);
    assert!(p.cycle_tab(PaneId::Left, -1, &|_| true));
    assert!(p.tabs(PaneId::Left)[1].active);
    assert!(p.close_tab(PaneId::Left, original, &|_| panic!(
        "inactive close is local"
    )));
    assert_eq!(p.tabs(PaneId::Left)[0].id, second);
    assert_eq!(p.tabs(PaneId::Right).len(), 1);
    assert_eq!(p.panes[1].generation, 0);
}

#[test]
fn parking_a_provisional_location_keeps_its_committed_history_branch() {
    let base = test_root();
    let mut p = Presenter::new(base.clone(), base.clone());
    let original = p.tabs(PaneId::Left)[0].id;
    for path in [base.join("a"), base.join("b")] {
        let navigation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
        finish(&mut p, PaneId::Left, navigation, vec![]);
    }
    let pending = base.join("c");
    begin(&mut p, PaneId::Left, pending.to_str().unwrap());
    assert!(p.new_tab(PaneId::Left, &|_| true));
    assert!(p.select_tab(PaneId::Left, original, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    finish(&mut p, PaneId::Left, navigation, vec![]);
    assert_eq!(
        p.panes[0].history,
        [base.join("a"), base.join("b"), pending]
    );
}

#[test]
fn default_titles_follow_each_tabs_folder_while_callback_identities_remain_stable() {
    let base = test_root();
    let mut p = Presenter::new(base.join("Alpha"), base.clone());
    let original = p.tabs(PaneId::Left)[0].id;
    assert_eq!(p.tabs(PaneId::Left)[0].title, "Alpha");
    let first = begin(
        &mut p,
        PaneId::Left,
        base.join("文档 e\u{301} 🧪").to_str().unwrap(),
    );
    finish(&mut p, PaneId::Left, first, vec![]);
    assert_eq!(p.tabs(PaneId::Left)[0].title, "文档 e\u{301} 🧪");
    assert_eq!(p.tabs(PaneId::Left)[0].id, original);
    assert!(p.new_tab(PaneId::Left, &|_| true));
    let second = begin(&mut p, PaneId::Left, base.join("Charlie").to_str().unwrap());
    finish(&mut p, PaneId::Left, second, vec![]);
    let tabs = p.tabs(PaneId::Left);
    assert_eq!(tabs[0].title, "文档 e\u{301} 🧪");
    assert_eq!(tabs[1].title, "Charlie");
    assert!(p.select_tab(PaneId::Left, original, &|_| true));
    let tabs = p.tabs(PaneId::Left);
    assert_eq!(tabs[0].title, "文档 e\u{301} 🧪");
    assert!(tabs[0].active);
    assert_eq!(tabs[1].title, "Charlie");
}
