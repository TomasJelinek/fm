use super::*;

fn snapshot(p: &mut Presenter, names: &[&str]) -> Arc<[Entry]> {
    let path = test_root();
    let navigation = begin(p, PaneId::Left, path.to_str().unwrap());
    let source: Arc<[Entry]> = names
        .iter()
        .enumerate()
        .map(|(row, name)| Entry::new(EntryId(row as u64 + 1), path.join(name), EntryKind::File))
        .collect::<Vec<_>>()
        .into();
    p.apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        entries: source.clone(),
        index: Arc::new(DirectoryIndex::from_entries(&source)),
    });
    source
}

fn query(p: &mut Presenter, text: &str) -> SearchRequest {
    let sent = RefCell::new(Vec::new());
    p.change_search_query(PaneId::Left, text, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    sent.into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Search(request) => Some(request),
            _ => None,
        })
        .expect("a committed source must be searched on the backend")
}

fn result(request: &SearchRequest, rows: &[usize], end: usize) -> Arc<SearchResult> {
    let entries: Vec<_> = rows
        .iter()
        .map(|row| request.source[*row].clone())
        .collect();
    Arc::new(SearchResult {
        source: request.source.clone(),
        filtered: request
            .source
            .iter()
            .enumerate()
            .map(|(source_row, _)| SearchMatch {
                source_row,
                display_prefix_end: 0,
            })
            .collect::<Vec<_>>()
            .into(),
        filtered_index: Arc::new(DirectoryIndex::from_entries(&request.source)),
        matches: rows
            .iter()
            .map(|row| SearchMatch {
                source_row: *row,
                display_prefix_end: end,
            })
            .collect::<Vec<_>>()
            .into(),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
    })
}

fn accept(p: &mut Presenter, request: &SearchRequest, result: Arc<SearchResult>) {
    p.apply(Event::SearchReady {
        pane: request.pane,
        navigation: request.navigation,
        revision: request.revision,
        result,
    });
}

#[test]
fn filter_projects_display_and_selection_without_replacing_native_snapshot_or_scan_scope() {
    let mut p = Presenter::new(test_root(), test_root());
    let source = snapshot(&mut p, &["alpha", "beta", "alpine", "gamma"]);
    let request = query(&mut p, "al");
    let found = result(&request, &[0, 2], 2);
    accept(&mut p, &request, found.clone());
    assert_eq!(p.panes[0].display_len(), 4);
    assert_eq!(p.panes[0].search_prefix_end(0), 2);
    assert_eq!(p.panes[0].search_prefix_end(1), 0);
    set_selected(&mut p, PaneId::Left, &[EntryId(1), EntryId(2)]);
    p.preferences.values.quick_search_filter = true;
    p.search_filter_changed();
    assert_eq!(p.panes[0].display_len(), 2);
    assert_eq!(p.panes[0].display_entry(1).unwrap().id, EntryId(3));
    assert!(Arc::ptr_eq(&p.panes[0].entries, &source));
    assert!(Arc::ptr_eq(
        &p.visible_directory(PaneId::Left).unwrap().entries,
        &source
    ));
    let sent = RefCell::new(Vec::new());
    p.flush_selection(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    let commands = sent.borrow();
    let Command::Selection(rebase) = commands.last().unwrap() else {
        panic!()
    };
    assert!(matches!(rebase.action, SelectionAction::Rebase));
    assert!(Arc::ptr_eq(rebase.view.as_ref().unwrap(), &found.index));
    assert!(rebase.seed.contains_path(&source[1].path));
    drop(commands);
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &|command| {
        let Command::Selection(request) = command else {
            panic!()
        };
        assert!(Arc::ptr_eq(request.view.as_ref().unwrap(), &found.index));
        true
    });
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        revision: p.panes[0].selection_revision,
        selection: Arc::new(SelectionSnapshot::from_bits(found.index.clone(), vec![3]).unwrap()),
        deferred: Default::default(),
    });
    assert!(!p.panes[0].selected.contains_path(&source[1].path));
    p.change_search_query(PaneId::Left, "", &|_| true);
    assert_eq!(p.panes[0].display_len(), 4);
    assert!(!p.panes[0].selected.contains_path(&source[1].path));
}

#[test]
fn delayed_queries_and_equal_but_unrelated_sources_cannot_retarget_search() {
    let mut p = Presenter::new(test_root(), test_root());
    snapshot(&mut p, &["alpha", "beta", "alpine"]);
    let old = query(&mut p, "al");
    let latest = query(&mut p, "b");
    accept(&mut p, &old, result(&old, &[0, 2], 2));
    assert!(p.panes[0].search.pending);
    let mut wrong_source = (*result(&latest, &[1], 1)).clone();
    wrong_source.source = latest.source.to_vec().into();
    accept(&mut p, &latest, Arc::new(wrong_source));
    assert!(p.panes[0].search.pending);
    accept(&mut p, &latest, result(&latest, &[1], 1));
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    accept(&mut p, &latest, result(&latest, &[0], 1));
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(2)),
        "duplicate completed results cannot replace an accepted view"
    );
    let context = p.panes[0].search.input_context;
    let previous_revision = latest.revision;
    assert!(p.navigate(PaneId::Left, test_root(), None, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    assert_eq!(p.panes[0].search.query, "b");
    assert_eq!(p.panes[0].search.input_context, context);
    let source: Arc<[Entry]> = vec![Entry::new(
        EntryId(50),
        test_root().join("beta"),
        EntryKind::File,
    )]
    .into();
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            entries: source.clone(),
            index: Arc::new(DirectoryIndex::from_entries(&source)),
        },
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    let next = sent
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Search(request) => Some(request),
            _ => None,
        })
        .unwrap();
    assert!(next.revision > previous_revision);
    assert!(Arc::ptr_eq(&next.source, &source));
    accept(&mut p, &latest, result(&latest, &[1], 1));
    assert!(p.panes[0].search.pending);
    accept(&mut p, &next, result(&next, &[0], 1));
    assert_eq!(p.panes[0].cursor, Some(EntryId(50)));
    assert!(p.new_tab(PaneId::Left, &|_| true));
    assert!(p.panes[0].search.query.is_empty());
    assert!(p.panes[0].search.input_context > context);
    assert!(p.search_revisions[0] > next.revision);
}

#[test]
fn streaming_and_queue_pressure_wait_for_immutable_source_and_retry_without_losing_query() {
    let mut p = Presenter::new(test_root(), test_root());
    p.preferences.values.quick_search_filter = true;
    let navigation = begin(&mut p, PaneId::Left, test_root().to_str().unwrap());
    let commands = RefCell::new(Vec::new());
    p.change_search_query(PaneId::Left, "al", &|command| {
        assert!(!matches!(command, Command::Search(_)));
        commands.borrow_mut().push(command);
        true
    });
    assert!(p.panes[0].search.pending);
    assert_eq!(p.panes[0].display_len(), 0);
    let source: Arc<[Entry]> = vec![entry(1, "alpha"), entry(2, "beta")].into();
    p.apply_with_sender(
        Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            entries: source.clone(),
            index: Arc::new(DirectoryIndex::from_entries(&source)),
        },
        &|_| false,
    );
    assert!(p.panes[0].search.pending);
    assert!(!p.bulk_selection(PaneId::Left, SelectionOperation::All, &|_| panic!()));
    let delivered = RefCell::new(None);
    p.flush_search(&|command| {
        *delivered.borrow_mut() = Some(command);
        true
    });
    let Some(Command::Search(request)) = delivered.into_inner() else {
        panic!()
    };
    assert!(Arc::ptr_eq(&request.source, &source));
    assert_eq!(request.query, "al");
    p.change_search_query(PaneId::Left, "", &|_| true);
    accept(&mut p, &request, result(&request, &[0], 2));
    assert_eq!(p.panes[0].display_len(), 2);
    assert!(!p.panes[0].search.pending);
    assert_eq!(p.panes[0].search_prefix_end(0), 0);
}

#[test]
fn match_navigation_is_bounded_and_no_match_cannot_open_old_cursor_properties() {
    let mut p = Presenter::new(test_root(), test_root());
    snapshot(&mut p, &["alpha", "beta", "alpine", "gamma", "al"]);
    let request = query(&mut p, "al");
    accept(&mut p, &request, result(&request, &[0, 2, 4], 2));
    for (key, id) in [
        (2, 1),
        (3, 3),
        (3, 5),
        (3, 5),
        (6, 1),
        (5, 5),
        (4, 1),
        (7, 5),
    ] {
        p.move_search_cursor(PaneId::Left, key);
        assert_eq!(p.panes[0].cursor, Some(EntryId(id)));
    }
    let request = query(&mut p, "missing");
    assert!(!p.panes[0].search_allows_activation());
    p.request_selection_properties(PaneId::Left, &|_| panic!("pending query must not activate"));
    accept(&mut p, &request, result(&request, &[], 0));
    assert_eq!(p.panes[0].cursor, Some(EntryId(5)));
    assert_eq!(p.panes[0].search.query, "missing");
    assert!(!p.panes[0].search_allows_activation());
    p.request_selection_properties(PaneId::Left, &|_| panic!("no match must not activate"));
    p.preferences.values.quick_search_filter = true;
    p.search_filter_changed();
    assert_eq!(
        p.focused_name(PaneId::Left),
        None,
        "a hidden stale basename cannot enter a command"
    );
    p.change_search_query(PaneId::Left, "", &|_| true);
    assert_eq!(p.focused_name(PaneId::Left), Some(PathBuf::from("al")));
}

#[test]
fn checkbox_and_late_saved_filter_apply_to_already_matched_queries() {
    for load_saved in [false, true] {
        let mut p = Presenter::new(test_root(), test_root());
        snapshot(&mut p, &["alpha", "beta", "alpine"]);
        let request = query(&mut p, "al");
        accept(&mut p, &request, result(&request, &[0, 2], 2));
        if load_saved {
            p.apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
                quick_search_filter: true,
                ..Default::default()
            }))));
        } else {
            p.change_search_filter(true, &|_| true);
        }
        assert_eq!(p.panes[0].display_len(), 2);
        assert_eq!(p.panes[0].search.query, "al");
        assert_eq!(p.panes[0].search_prefix_end(1), 2);
        assert!(p.panes[0].selection_rebase_pending);
        p.change_search_filter(false, &|_| true);
        assert_eq!(p.panes[0].display_len(), 3);
        assert_eq!(p.panes[0].search.query, "al");
    }
}

#[test]
fn escape_cancel_retries_after_queue_pressure_and_never_cancels_a_newer_query() {
    let mut p = Presenter::new(test_root(), test_root());
    snapshot(&mut p, &["alpha", "beta"]);
    let first = query(&mut p, "al");
    p.change_search_query(PaneId::Left, "", &|_| false);
    let cancelled = p.search_revisions[0];
    let retry = RefCell::new(Vec::new());
    p.flush_search(&|command| {
        retry.borrow_mut().push(command);
        true
    });
    assert!(
        retry.borrow().iter().any(|command| matches!(command,
        Command::CancelSearch { pane: PaneId::Left, revision } if *revision == cancelled)),
        "Escape cancellation must be retried even though the query is now empty"
    );
    assert!(cancelled > first.revision);
    query(&mut p, "b");
    p.change_search_query(PaneId::Left, "", &|_| false);
    p.change_search_query(PaneId::Left, "a", &|_| false);
    let delivered = RefCell::new(Vec::new());
    p.flush_search(&|command| {
        delivered.borrow_mut().push(command);
        true
    });
    let delivered = delivered.into_inner();
    let latest = delivered
        .iter()
        .find_map(|command| match command {
            Command::Search(request) => Some(request.revision),
            _ => None,
        })
        .unwrap();
    assert!(delivered.iter().all(|command| !matches!(command,
        Command::CancelSearch { revision, .. } if *revision >= latest)));
    p.flush_search(&|_| panic!("a successful newer search supersedes every stale cancellation"));
    p.change_search_query(PaneId::Left, "", &|_| false);
    assert!(p.new_tab(PaneId::Left, &|command| !matches!(
        command,
        Command::CancelSearch { .. }
    )));
    let latest_context = p.search_revisions[0];
    let retry = RefCell::new(Vec::new());
    p.flush_search(&|command| {
        retry.borrow_mut().push(command);
        true
    });
    assert!(
        retry.borrow().iter().any(|command| matches!(command,
        Command::CancelSearch { revision, .. } if *revision == latest_context)),
        "a tab replacement must not discard a pending physical-pane cancellation"
    );
}

#[test]
fn failed_refresh_searches_the_restored_canonical_generation_and_keeps_its_error() {
    let mut p = Presenter::new(test_root(), test_root());
    let source = snapshot(&mut p, &["alpha", "beta"]);
    let request = query(&mut p, "al");
    p.apply(Event::SearchFailed {
        pane: PaneId::Left,
        navigation: request.navigation,
        revision: request.revision,
        error: FsError::from_io(
            "search",
            &test_root(),
            io::Error::other("search unavailable"),
        ),
    });
    assert!(p.navigate(PaneId::Left, test_root(), None, &|_| true));
    let failed = NavigationId(p.panes[0].generation);
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::ListingFailed {
            pane: PaneId::Left,
            navigation: failed,
            error: FsError::from_io(
                "refresh",
                &test_root(),
                io::Error::new(io::ErrorKind::PermissionDenied, "refresh denied"),
            ),
        },
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    let retry = sent
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Search(request) => Some(request),
            _ => None,
        })
        .unwrap();
    assert_eq!(retry.navigation, request.navigation);
    assert_ne!(retry.navigation, failed);
    assert!(Arc::ptr_eq(&retry.source, &source));
    accept(&mut p, &retry, result(&retry, &[0], 2));
    assert!(p.panes[0].search_allows_activation());
    assert!(p.panes[0].error.contains("refresh denied"));
}

#[test]
fn unchanged_query_refresh_and_sort_keep_a_still_matching_raw_cursor() {
    for resort in [false, true] {
        let mut p = Presenter::new(test_root(), test_root());
        let source = snapshot(&mut p, &["al-a", "al-b", "al-c"]);
        let request = query(&mut p, "al");
        accept(&mut p, &request, result(&request, &[0, 1, 2], 2));
        p.move_search_cursor(PaneId::Left, 3);
        assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
        if resort {
            assert!(p.sort(PaneId::Left, SortColumn::Name, &|_| true));
        } else {
            assert!(p.refresh_if_changed(PaneId::Left, request.navigation, &|_| true));
        }
        let refreshed: Arc<[Entry]> =
            vec![source[2].clone(), source[0].clone(), source[1].clone()].into();
        let sent = RefCell::new(Vec::new());
        p.apply_with_sender(
            Event::ListingFinished {
                pane: PaneId::Left,
                navigation: NavigationId(p.panes[0].generation),
                index: Arc::new(DirectoryIndex::from_entries(&refreshed)),
                entries: refreshed,
            },
            &|command| {
                sent.borrow_mut().push(command);
                true
            },
        );
        let refreshed = sent
            .into_inner()
            .into_iter()
            .find_map(|command| match command {
                Command::Search(request) => Some(request),
                _ => None,
            })
            .unwrap();
        accept(&mut p, &refreshed, result(&refreshed, &[0, 1, 2], 2));
        assert_eq!(
            p.panes[0].cursor,
            Some(EntryId(2)),
            "an unchanged query must preserve the raw focused match after refresh/sort"
        );
        if !resort {
            assert!(
                p.panes[0].preserve_viewport,
                "a watch refresh must preserve its scroll intent"
            );
        }
        let changed = query(&mut p, "al-");
        accept(&mut p, &changed, result(&changed, &[0, 1, 2], 3));
        assert_eq!(
            p.panes[0].cursor,
            Some(EntryId(3)),
            "a changed query still starts at its first match"
        );
    }
}

#[test]
fn filter_checkbox_keeps_a_current_match_and_replaces_only_a_hidden_cursor() {
    let mut p = Presenter::new(test_root(), test_root());
    snapshot(&mut p, &["al-a", "al-b", "other"]);
    let request = query(&mut p, "al");
    accept(&mut p, &request, result(&request, &[0, 1], 2));
    p.move_search_cursor(PaneId::Left, 3);
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    p.change_search_filter(true, &|_| true);
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(2)),
        "changing the display mode does not change a still-visible focused match"
    );
    p.change_search_filter(false, &|_| true);
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    p.select_row(PaneId::Left, p.panes[0].parent_offset() + 2, false, &|_| {
        true
    });
    assert_eq!(p.panes[0].cursor, Some(EntryId(3)));
    p.change_search_filter(true, &|_| true);
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(1)),
        "a newly hidden cursor falls back to the first match"
    );
}
