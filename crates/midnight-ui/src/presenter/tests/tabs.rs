use super::*;

fn saved_tabs(paths: &[&str], active: usize) -> PaneTabPreferences {
    PaneTabPreferences {
        paths: paths.iter().map(|path| test_root().join(path)).collect(),
        active,
    }
}

#[test]
fn saved_tabs_restore_order_active_positions_and_requery_only_active_locations() {
    let mut p = Presenter::new(
        test_root().join("initial-left"),
        test_root().join("initial-right"),
    );
    let old_ids = [p.tabs(PaneId::Left)[0].id, p.tabs(PaneId::Right)[0].id];
    let saved = [
        saved_tabs(&["first", "文档 e\u{301} 🧪", "third"], 1),
        saved_tabs(&["right-a", "right-b"], 0),
    ];
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(UserPreferences {
            tabs: saved.clone(),
            ..Default::default()
        })),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    for pane in [PaneId::Left, PaneId::Right] {
        let tabs = p.tabs(pane);
        let expected = &saved[pane.index()];
        assert_eq!(
            tabs.iter().map(|tab| tab.path.clone()).collect::<Vec<_>>(),
            expected.paths
        );
        assert_eq!(
            tabs.iter().position(|tab| tab.active),
            Some(expected.active)
        );
        assert!(tabs.iter().all(|tab| !old_ids.contains(&tab.id)));
        assert!(p.panes[pane.index()].entries.is_empty());
        assert!(p.panes[pane.index()].selected.is_empty());
        assert!(p.panes[pane.index()].history.is_empty());
    }
    assert_eq!(
        sent.borrow()
            .iter()
            .filter(|command| matches!(command, Command::Navigate { .. }))
            .count(),
        2
    );
}

#[test]
fn local_tab_edits_win_over_late_load_while_untouched_panel_restores() {
    let mut p = Presenter::new(test_root().join("local"), test_root());
    assert!(p.new_tab(PaneId::Left, &|_| true));
    let local = p.tabs(PaneId::Left);
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(UserPreferences {
            tabs: [
                saved_tabs(&["saved-left"], 0),
                saved_tabs(&["right-a", "right-b"], 1),
            ],
            theme: PreferenceTheme::Dark,
            ..Default::default()
        })),
        &|_| true,
    );
    assert_eq!(p.tabs(PaneId::Left), local);
    assert_eq!(p.tabs(PaneId::Right).len(), 2);
    assert!(p.tabs(PaneId::Right)[1].active);
    assert_eq!(p.preferences.values.tabs[0].paths.len(), 2);
    assert_eq!(p.preferences.values.theme, PreferenceTheme::Dark);
}

#[test]
fn startup_path_override_affects_only_its_panel_and_early_navigation_wins() {
    let mut p = Presenter::new(test_root().join("explicit"), test_root());
    p.set_tab_restore_overrides([true, false]);
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(UserPreferences {
            tabs: [
                saved_tabs(&["saved-left"], 0),
                saved_tabs(&["saved-right"], 0),
            ],
            ..Default::default()
        })),
        &|_| true,
    );
    assert_eq!(p.tabs(PaneId::Left)[0].path, test_root().join("explicit"));
    assert_eq!(
        p.tabs(PaneId::Right)[0].path,
        test_root().join("saved-right")
    );
    let mut p = Presenter::new(test_root(), test_root());
    assert!(p.navigate(
        PaneId::Left,
        test_root().join("early-user-path"),
        None,
        &|_| true
    ));
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(UserPreferences {
            tabs: [
                saved_tabs(&["saved-left"], 0),
                saved_tabs(&["saved-right"], 0),
            ],
            ..Default::default()
        })),
        &|_| true,
    );
    assert_eq!(
        p.tabs(PaneId::Left)[0].path,
        test_root().join("early-user-path")
    );
}

#[test]
fn closed_tabs_and_the_active_index_are_saved_and_survive_restart() {
    let mut p = Presenter::new(test_root().join("one"), test_root());
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(UserPreferences::default())),
        &|_| true,
    );
    let saved = RefCell::new(None);
    let send = |command| {
        if let Command::SavePreferences { preferences, .. } = command {
            *saved.borrow_mut() = Some(preferences);
        }
        true
    };
    assert!(p.new_tab(PaneId::Left, &send));
    assert!(p.navigate(PaneId::Left, test_root().join("two"), None, &send));
    assert!(p.new_tab(PaneId::Left, &send));
    assert!(p.navigate(PaneId::Left, test_root().join("three"), None, &send));
    let tabs = p.tabs(PaneId::Left);
    assert!(p.select_tab(PaneId::Left, tabs[0].id, &send));
    assert!(p.close_tab(PaneId::Left, tabs[1].id, &send));
    let preferences = saved
        .into_inner()
        .expect("tab changes must enqueue a preference save");
    assert_eq!(preferences.tabs[0], saved_tabs(&["one", "three"], 0));
    let mut restarted = Presenter::new(test_root(), test_root());
    restarted.apply_with_sender(Event::PreferencesLoaded(Ok(preferences)), &|_| true);
    let tabs = restarted.tabs(PaneId::Left);
    assert_eq!(
        tabs.iter().map(|tab| tab.path.clone()).collect::<Vec<_>>(),
        saved_tabs(&["one", "three"], 0).paths
    );
    assert!(tabs[0].active);
}

#[test]
fn rejected_restore_retries_without_discarding_saved_tabs_or_current_view() {
    let mut p = Presenter::new(test_root().join("initial"), test_root());
    let original = p.tabs(PaneId::Left);
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(UserPreferences {
            tabs: [saved_tabs(&["restored"], 0), PaneTabPreferences::default()],
            ..Default::default()
        })),
        &|_| false,
    );
    assert_eq!(p.tabs(PaneId::Left), original);
    assert!(p.navigate(PaneId::Left, test_root().join("initial"), None, &|_| true));
    p.flush_loaded_preferences(&|_| true);
    assert_eq!(p.tabs(PaneId::Left)[0].path, test_root().join("restored"));
}

#[test]
fn initial_panel_paths_are_saved_after_load_even_without_a_tab_edit() {
    let mut p = Presenter::new(
        test_root().join("initial-left"),
        test_root().join("initial-right"),
    );
    p.set_tab_restore_overrides([false, false]);
    assert!(p.needs_initial_preferences());
    let saved = RefCell::new(None);
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(UserPreferences::default())),
        &|command| {
            if let Command::SavePreferences { preferences, .. } = command {
                *saved.borrow_mut() = Some(preferences);
            }
            true
        },
    );
    assert_eq!(
        saved.into_inner().unwrap().tabs,
        [
            saved_tabs(&["initial-left"], 0),
            saved_tabs(&["initial-right"], 0)
        ]
    );
}

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
