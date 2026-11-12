//! Regressions at the real presenter command/publication boundary.
use super::*;
use std::cell::RefCell;

fn fixture() -> Presenter {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tree-filter-fixture");
    let mut p = Presenter::new(path.clone(), path.clone());
    p.preferences.loaded = true;
    let commands = RefCell::new(Vec::new());
    p.navigate(PaneId::Left, path.clone(), None, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    let navigation = NavigationId(p.panes[0].generation);
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(42),
        path.join("folder"),
        EntryKind::Directory,
    )]
    .into();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    p.apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        entries: entries.clone(),
        index: index.clone(),
    });
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation,
        entries,
        index,
    };
    let identity = p.panes[0].row_identity(&root.entries[0]);
    p.disclose_tree(PaneId::Left, &identity, &|_| true);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(root, 1)))),
        &|_| true,
    );
    p
}

fn publication(root: TreeRootSnapshot, revision: u64) -> TreePublication {
    let mut parent = root.entries[0].clone();
    parent.id = EntryId(7);
    let mut child = Entry::new(EntryId(11), parent.path.join("child.txt"), EntryKind::File);
    child.metadata.size = Some(5);
    let rows = vec![
        TreeDisplayRow {
            id: TreeRowId(7),
            entry: parent,
            parent: None,
            depth: 0,
            disclosure: TreeDisclosure::Expanded,
            expanded: true,
            selectable: true,
            navigable: true,
            context_only: false,
            error: None,
            error_details: None,
            original_root_entry: Some(EntryId(42)),
            prefix_end: 0,
            search_match: false,
        },
        TreeDisplayRow {
            id: TreeRowId(11),
            entry: child,
            parent: Some(TreeRowId(7)),
            depth: 1,
            disclosure: TreeDisclosure::None,
            expanded: false,
            selectable: true,
            navigable: false,
            context_only: false,
            error: None,
            error_details: None,
            original_root_entry: None,
            prefix_end: 0,
            search_match: false,
        },
    ];
    let stamp = TreeViewStamp {
        pane: root.pane,
        navigation: root.navigation,
        session: TreeSessionId(5),
        revision: TreeRevision(revision),
    };
    let view =
        Arc::new(TreeProjection::new(root, stamp, TreeLayoutRevision(revision), rows).unwrap());
    let selection = Arc::new(
        TreeSelectionSnapshot::from_bits(view.clone(), vec![0; view.rows.len().div_ceil(64)])
            .unwrap(),
    );
    TreePublication::new(view, SelectionRevision(0), selection).unwrap()
}

fn validate(p: &mut Presenter) {
    p.open_panel_filter(PaneId::Left);
    assert!(
        p.filter_dialog.open,
        "an adopted tree supports its persistent filter dialog"
    );
    let sent = RefCell::new(None);
    p.submit_panel_filter(
        PaneId::Left,
        PanelFilter {
            enabled: true,
            mode: FilterMode::Masks,
            expression: "*.txt".into(),
            case_sensitive: false,
        },
        &|command| {
            if let Command::ValidatePanelFilter(request) = command {
                *sent.borrow_mut() = Some(request);
            }
            true
        },
    );
    p.apply(Event::PanelFilterValidated {
        request: sent.into_inner().unwrap(),
        result: Ok(()),
        diagnostic: None,
    });
}

#[test]
fn tree_filter_ok_waits_for_application_receipt() {
    let mut p = fixture();
    let original = p.panes[0].tree_view().unwrap().clone();
    validate(&mut p);
    let sent = RefCell::new(Vec::new());
    p.flush_filters(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(!p.filter_dialog.open);
    assert!(!p.preferences.values.panel_filters[0].enabled);
    assert!(p.preferences.values.panel_filter_history.is_empty());
    assert!(!p.panes[0].search.panel_filter.enabled);
    assert!(Arc::ptr_eq(&original, p.panes[0].tree_view().unwrap()));
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::Tree(_)))
    );
    assert!(!sent.borrow().iter().any(|command| matches!(
        command,
        Command::SavePreferences { .. } | Command::Search(_) | Command::Selection(_)
    )));
}

#[test]
fn tree_filter_cancel_and_enqueue_failure_keep_the_draft_uncommitted() {
    let mut p = fixture();
    validate(&mut p);
    p.cancel_panel_filter();
    p.flush_filters(&|_| panic!("cancelled validation cannot enqueue"));
    validate(&mut p);
    p.flush_filters(&|_| false);
    assert!(p.filter_dialog.open);
    assert!(!p.filter_dialog.busy);
    assert!(!p.filter_dialog.error.is_empty());
    assert_eq!(p.filter_dialog.draft.expression, "*.txt");
    assert!(!p.preferences.values.panel_filters[0].enabled);
}

#[test]
fn tree_filter_sort_does_not_navigate_or_eagerly_change_options() {
    let mut p = fixture();
    let sort = p.panes[0].sort;
    let original = p.panes[0].tree_view().unwrap().clone();
    let sent = RefCell::new(Vec::new());
    assert!(p.sort(PaneId::Left, SortColumn::Size, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    assert_eq!(p.panes[0].sort, sort);
    assert!(Arc::ptr_eq(&original, p.panes[0].tree_view().unwrap()));
    assert!(
        sent.borrow()
            .iter()
            .all(|command| matches!(command, Command::Tree(_)))
    );
}

fn enqueue(p: &mut Presenter) -> TreeReprojectRequest {
    validate(p);
    let sent = RefCell::new(None);
    p.flush_filters(&|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Reproject(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    sent.into_inner()
        .expect("validated tree options are queued")
}

fn receipt(request: &TreeReprojectRequest) -> Event {
    Event::Tree(Box::new(TreeEvent::Reprojected {
        previous: request.view.stamp,
        request: request.request,
        options: request.options.clone(),
        publication: publication(request.view.root.clone(), request.view.stamp.revision.0 + 1),
    }))
}

#[test]
fn tree_filter_matching_receipt_adopts_and_saves_once_after_commit() {
    let mut p = fixture();
    let request = enqueue(&mut p);
    assert!(!p.panes[0].tree_input_ready());
    assert!(!p.sort(PaneId::Left, SortColumn::Size, &|_| panic!(
        "a second options request must stay bounded"
    )));
    assert!(Arc::ptr_eq(
        &request.selection,
        &p.panes[0].tree.publication.as_ref().unwrap().selection
    ));
    assert!(!p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(11)),
        &|_| panic!("pending options fence selection")
    ));
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(receipt(&request), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert_eq!(
        p.preferences.values.panel_filters[0],
        request.options.filter
    );
    assert_eq!(p.panes[0].search.panel_filter, request.options.filter);
    assert!(p.panes[0].tree_input_ready());
    assert!(p.panes[0].search_allows_activation());
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. }))
    );
    assert!(sent.borrow().iter().any(|command| matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Commit(_)))));
    assert!(
        !sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::Search(_) | Command::Selection(_)))
    );
    let revision = p.preferences.revision;
    p.apply_with_sender(receipt(&request), &|command| {
        assert!(!matches!(command, Command::SavePreferences { .. }));
        true
    });
    assert_eq!(p.preferences.revision, revision);
}

#[test]
fn tree_filter_worker_rejection_preserves_previous_filter_after_branch_publication() {
    let mut p = fixture();
    let request = enqueue(&mut p);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            request.view.root.clone(),
            2,
        )))),
        &|_| true,
    );
    assert!(!p.panes[0].tree_input_ready());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::ReprojectFailed {
            stamp: request.view.stamp,
            request: request.request,
            error: TreeDiagnostic::new("The tree changed; try again."),
            retryable: true,
        })),
        &|_| true,
    );
    assert!(
        p.panes[0].tree_input_ready(),
        "failure correlates with the captured stamp even after ordinary publication"
    );
    assert!(!p.preferences.values.panel_filters[0].enabled);
    assert!(!p.panes[0].search.panel_filter.enabled);
    assert!(!p.panes[0].error.is_empty());
    p.open_panel_filter(PaneId::Left);
    assert_eq!(p.filter_dialog.draft.expression, "*.txt");
}

#[test]
fn tree_filter_receipts_cannot_change_replaced_panes_or_newer_intents() {
    let mut p = fixture();
    let request = enqueue(&mut p);
    let mut foreign = request.clone();
    foreign.options.filter.expression = "*.bin".into();
    p.apply_with_sender(receipt(&foreign), &|_| true);
    assert!(!p.preferences.values.panel_filters[0].enabled);
    assert!(!p.panes[0].tree_input_ready());
    let path = p.panes[0].path.join("replacement");
    assert!(p.navigate(PaneId::Left, path, None, &|_| true));
    p.apply_with_sender(receipt(&request), &|_| true);
    assert!(p.panes[0].tree_view().is_none());
    assert!(!p.preferences.values.panel_filters[0].enabled);
    assert!(!p.filter_dialog.open);
}

#[test]
fn tree_filter_waits_for_settled_commit_and_selection_without_enqueueing() {
    let mut p = fixture();
    p.panes[0].tree.commit_pending = true;
    validate(&mut p);
    p.flush_filters(&|_| panic!("options cannot precede Commit"));
    assert!(p.filter_dialog.open);
    p.panes[0].tree.commit_pending = false;
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(11)),
        &|_| true
    ));
    validate(&mut p);
    p.flush_filters(&|_| panic!("options cannot use an unacknowledged selection"));
    assert!(p.filter_dialog.open);
    assert!(!p.preferences.values.panel_filters[0].enabled);
}

#[test]
fn tree_filter_preferences_use_tree_route_and_keep_unrelated_edits_on_failure() {
    let mut p = fixture();
    let mut preferences = p.preferences.values.clone();
    preferences.quick_search_filter = !preferences.quick_search_filter;
    preferences.panel_filters[0] = PanelFilter {
        enabled: true,
        expression: "*.txt".into(),
        ..Default::default()
    };
    let quick_search = preferences.quick_search_filter;
    p.update_preferences(preferences, &|_| true);
    assert!(
        !p.preferences.values.panel_filters[0].enabled,
        "unacknowledged tree options cannot be saved"
    );
    assert!(!p.panes[0].search.panel_filter.enabled);
    let sent = RefCell::new(None);
    p.flush_search(&|command| {
        assert!(!matches!(
            command,
            Command::Search(_) | Command::Selection(_)
        ));
        if let Command::Tree(tree) = command
            && let TreeCommand::Reproject(request) = *tree
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    let request = sent.into_inner().unwrap();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::ReprojectFailed {
            stamp: request.view.stamp,
            request: request.request,
            error: TreeDiagnostic::new("busy"),
            retryable: false,
        })),
        &|_| true,
    );
    assert_eq!(p.preferences.values.quick_search_filter, quick_search);
    assert!(!p.preferences.values.panel_filters[0].enabled);
}

#[test]
fn tree_filter_clear_keeps_expression_until_matching_receipt() {
    let mut p = fixture();
    let request = enqueue(&mut p);
    p.apply_with_sender(receipt(&request), &|_| true);
    let sent = RefCell::new(None);
    p.clear_panel_filter(PaneId::Left, &|command| {
        if let Command::Tree(tree) = command
            && let TreeCommand::Reproject(request) = *tree
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    assert!(p.preferences.values.panel_filters[0].enabled);
    let clear = sent.into_inner().unwrap();
    assert!(!clear.options.filter.enabled);
    assert_eq!(clear.options.filter.expression, "*.txt");
    p.apply_with_sender(receipt(&clear), &|_| true);
    assert!(!p.preferences.values.panel_filters[0].enabled);
    assert_eq!(p.preferences.values.panel_filters[0].expression, "*.txt");
}

#[test]
fn tree_filter_explicit_ok_supersedes_an_older_unsubmitted_preference() {
    let mut p = fixture();
    let mut preferences = p.preferences.values.clone();
    preferences.panel_filters[0] = PanelFilter {
        enabled: true,
        expression: "*.bin".into(),
        ..Default::default()
    };
    p.update_preferences(preferences, &|_| true);
    let request = enqueue(&mut p);
    p.apply_with_sender(receipt(&request), &|command| {
        assert!(!matches!(command, Command::Tree(tree) if matches!(tree.as_ref(), TreeCommand::Reproject(_))), "an older preference must not override committed OK");
        true
    });
    assert_eq!(p.preferences.values.panel_filters[0].expression, "*.txt");
}

#[test]
fn tree_filter_draft_uses_latest_settled_publication_of_the_same_root() {
    let mut p = fixture();
    validate(&mut p);
    let root = p.panes[0].tree_view().unwrap().root.clone();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(root, 2)))),
        &|_| true,
    );
    let current = p.panes[0].tree_view().unwrap().clone();
    let captured = RefCell::new(None);
    p.flush_filters(&|command| {
        if let Command::Tree(tree) = command
            && let TreeCommand::Reproject(request) = *tree
        {
            *captured.borrow_mut() = Some(request);
        }
        true
    });
    assert!(Arc::ptr_eq(&captured.into_inner().unwrap().view, &current));
    assert!(!p.filter_dialog.open);
}

#[test]
fn tree_filter_draft_from_flat_view_cannot_retarget_an_adopted_tree() {
    let mut p = fixture();
    let tree = mem::take(&mut p.panes[0].tree);
    validate(&mut p);
    p.panes[0].tree = tree;
    p.flush_filters(&|_| panic!("a flat draft cannot retarget a later tree"));
    assert!(p.filter_dialog.open);
    assert!(!p.filter_dialog.error.is_empty());
    assert!(!p.preferences.values.panel_filters[0].enabled);
}

#[test]
fn tree_filter_failed_draft_survives_an_unrelated_successful_sort() {
    let mut p = fixture();
    let request = enqueue(&mut p);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::ReprojectFailed {
            stamp: request.view.stamp,
            request: request.request,
            error: TreeDiagnostic::new("busy"),
            retryable: false,
        })),
        &|_| true,
    );
    let captured = RefCell::new(None);
    assert!(p.sort(PaneId::Left, SortColumn::Size, &|command| {
        if let Command::Tree(tree) = command
            && let TreeCommand::Reproject(request) = *tree
        {
            *captured.borrow_mut() = Some(request);
        }
        true
    }));
    p.apply_with_sender(receipt(&captured.into_inner().unwrap()), &|_| true);
    assert_eq!(p.panes[0].sort.column, SortColumn::Size);
    p.open_panel_filter(PaneId::Left);
    assert_eq!(p.filter_dialog.draft.expression, "*.txt");
    assert!(!p.preferences.values.panel_filters[0].enabled);
}

#[test]
fn tree_prefix_input_is_retained_and_enqueues_reprojection() {
    let mut p = fixture();
    let original = p.panes[0].tree_view().unwrap().clone();
    let sent = RefCell::new(Vec::new());
    p.change_search_query(PaneId::Left, "ch", &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert_eq!(p.panes[0].search.query, "ch");
    assert!(p.panes[0].search.pending);
    assert!(!p.panes[0].search_allows_activation());
    assert!(Arc::ptr_eq(&original, p.panes[0].tree_view().unwrap()));
    assert!(sent.borrow().iter().any(|command| matches!(command,
        Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Reproject(_)))));
    assert!(
        sent.borrow()
            .iter()
            .all(|command| !matches!(command, Command::Search(_)))
    );
}

#[test]
fn tree_prefix_full_queue_and_escape_keep_latest_input() {
    let mut p = fixture();
    for query in ["c", "ch", "chi", ""] {
        p.change_search_query(PaneId::Left, query, &|_| false);
        assert_eq!(p.panes[0].search.query, query);
        assert!(
            p.panes[0].error.is_empty(),
            "automatic backpressure stays pending"
        );
    }
    let sent = RefCell::new(Vec::new());
    p.flush_search(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(
        !sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::Search(_)))
    );
    assert_eq!(p.panes[0].search.query, "");
}

fn prefix_request(p: &mut Presenter, query: &str) -> TreeReprojectRequest {
    let sent = RefCell::new(None);
    p.change_search_query(PaneId::Left, query, &|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Reproject(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    sent.into_inner().expect("tree prefix request")
}

fn prefix_receipt(request: &TreeReprojectRequest, matches: &[usize]) -> Event {
    let base = publication(request.view.root.clone(), request.view.stamp.revision.0 + 1);
    let mut rows = base.view.rows.to_vec();
    for position in matches {
        rows[*position].search_match = true;
        rows[*position].prefix_end = request.options.search.query.len();
    }
    if request.options.search.filter_matches && !request.options.search.query.is_empty() {
        if matches.is_empty() {
            rows.clear();
        } else if !matches.contains(&0) {
            rows[0].context_only = true;
            rows[0].selectable = false;
        }
    }
    let view = Arc::new(
        TreeProjection::new(
            base.view.root.clone(),
            base.view.stamp,
            base.view.layout_revision,
            rows,
        )
        .unwrap(),
    );
    let selection = Arc::new(
        TreeSelectionSnapshot::from_bits(view.clone(), vec![0; view.rows.len().div_ceil(64)])
            .unwrap(),
    );
    Event::Tree(Box::new(TreeEvent::Reprojected {
        previous: request.view.stamp,
        request: request.request,
        options: request.options.clone(),
        publication: TreePublication::new(view, SelectionRevision(0), selection).unwrap(),
    }))
}

#[test]
fn tree_prefix_latest_query_and_escape_supersede_inflight_first_match() {
    let mut p = fixture();
    p.panes[0].cursor = Some(EntryId(7));
    p.panes[0].parent_cursor = false;
    let request = prefix_request(&mut p, "c");
    for query in ["ch", "chi", ""] {
        p.change_search_query(PaneId::Left, query, &|_| panic!("one inflight request"));
    }
    let sent = RefCell::new(None);
    p.apply_with_sender(prefix_receipt(&request, &[1]), &|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Reproject(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(7)),
        "stale query must not focus its first match"
    );
    assert_eq!(p.panes[0].search.query, "");
    assert_eq!(p.panes[0].search_prefix_end(1), 0);
    assert!(!p.panes[0].search_allows_activation());
    let clear = sent
        .into_inner()
        .expect("latest Escape is delivered after receipt");
    assert!(clear.options.search.query.is_empty());
    p.apply_with_sender(prefix_receipt(&clear, &[]), &|_| true);
    assert_eq!(p.panes[0].cursor, Some(EntryId(7)));
    assert!(p.panes[0].search_allows_activation());
}

#[test]
fn tree_prefix_navigation_uses_backend_matches_and_zero_matches_fence_activation() {
    let mut p = fixture();
    let request = prefix_request(&mut p, "c");
    p.apply_with_sender(prefix_receipt(&request, &[1]), &|_| true);
    assert_eq!(p.panes[0].search_match_count(), 1);
    assert_eq!(p.panes[0].cursor, Some(EntryId(11)));
    assert!(p.panes[0].search_allows_activation());
    for key in [2, 3, 4, 5, 6, 7] {
        p.move_search_cursor(PaneId::Left, key);
        assert_eq!(p.panes[0].cursor, Some(EntryId(11)));
    }
    let request = prefix_request(&mut p, "zz");
    p.apply_with_sender(prefix_receipt(&request, &[]), &|_| true);
    assert_eq!(p.panes[0].search_match_count(), 0);
    assert!(!p.panes[0].search_allows_activation());
    p.move_search_cursor(PaneId::Left, 6);
    assert!(!p.panes[0].search_allows_activation());
}

#[test]
fn tree_prefix_waits_for_selection_and_retries_branch_rejection() {
    let mut p = fixture();
    p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(11)),
        &|_| true,
    );
    p.change_search_query(PaneId::Left, "c", &|_| {
        panic!("selection must settle first")
    });
    let published = p.panes[0].tree.publication.as_ref().unwrap().clone();
    let selection =
        Arc::new(TreeSelectionSnapshot::from_bits(published.view.clone(), vec![2]).unwrap());
    let sent = RefCell::new(None);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            view: published.view,
            revision: SelectionRevision(1),
            selection,
        })),
        &|command| {
            if let Command::Tree(command) = command
                && let TreeCommand::Reproject(request) = *command
            {
                *sent.borrow_mut() = Some(request);
            }
            true
        },
    );
    let request = sent
        .into_inner()
        .expect("settled selection releases prefix request");
    assert_eq!(request.selection_revision, SelectionRevision(1));
    let mut branch = publication(request.view.root.clone(), 2);
    branch.selection_revision = SelectionRevision(1);
    p.apply_with_sender(Event::Tree(Box::new(TreeEvent::Published(branch))), &|_| {
        true
    });
    let sent = RefCell::new(None);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::ReprojectFailed {
            stamp: request.view.stamp,
            request: request.request,
            error: TreeDiagnostic::new("Tree changed"),
            retryable: true,
        })),
        &|command| {
            if let Command::Tree(command) = command
                && let TreeCommand::Reproject(request) = *command
            {
                *sent.borrow_mut() = Some(request);
            }
            true
        },
    );
    let retry = sent
        .into_inner()
        .expect("branch advancement retries latest prefix");
    assert_eq!(retry.view.stamp.revision, TreeRevision(2));
    assert_eq!(retry.options.search.query, "c");
}

#[test]
fn tree_prefix_options_preview_cancel_preserves_latest_mode_while_inflight() {
    let mut p = fixture();
    let request = prefix_request(&mut p, "c");
    p.begin_options();
    let mut values = p.preferences.values.clone();
    values.quick_search_filter = true;
    p.update_preferences(values, &|_| true);
    p.cancel_options(&|_| true);
    assert!(!p.preferences.values.quick_search_filter);
    p.apply_with_sender(prefix_receipt(&request, &[1]), &|command| {
        assert!(!matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Reproject(_))),
            "cancelled preview already matches inflight request");
        true
    });
    assert!(!p.panes[0].search.filter);
    assert!(!p.panes[0].search.pending);
}

#[test]
fn tree_prefix_preference_preview_and_cancel_reproject_active_query() {
    let mut p = fixture();
    let request = prefix_request(&mut p, "c");
    p.apply_with_sender(prefix_receipt(&request, &[1]), &|_| true);
    p.begin_options();
    let mut values = p.preferences.values.clone();
    values.quick_search_filter = true;
    p.update_preferences(values, &|_| true);
    assert!(p.panes[0].search.pending);
    let sent = RefCell::new(None);
    p.flush_search(&|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Reproject(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    let filtered = sent
        .into_inner()
        .expect("quick-filter preview reprojects tree");
    assert_eq!(filtered.options.search.query, "c");
    assert!(filtered.options.search.filter_matches);
    p.apply_with_sender(prefix_receipt(&filtered, &[1]), &|_| true);
    p.cancel_options(&|_| true);
    let sent = RefCell::new(None);
    p.flush_search(&|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Reproject(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    let restored = sent.into_inner().expect("Cancel restores jump mode");
    assert_eq!(restored.options.search.query, "c");
    assert!(!restored.options.search.filter_matches);
}

#[test]
fn tree_prefix_sort_and_filter_clear_preserve_active_transient_query() {
    let mut p = fixture();
    let filter = enqueue(&mut p);
    p.apply_with_sender(receipt(&filter), &|_| true);
    let request = prefix_request(&mut p, "c");
    assert_eq!(request.options.filter.expression, "*.txt");
    p.apply_with_sender(prefix_receipt(&request, &[1]), &|_| true);
    let sent = RefCell::new(None);
    assert!(p.sort(PaneId::Left, SortColumn::Size, &|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Reproject(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    }));
    let sorted = sent.into_inner().unwrap();
    assert_eq!(sorted.options.search.query, "c");
    p.apply_with_sender(prefix_receipt(&sorted, &[1]), &|_| true);
    let sent = RefCell::new(None);
    p.clear_panel_filter(PaneId::Left, &|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Reproject(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    let cleared = sent.into_inner().unwrap();
    assert_eq!(cleared.options.search.query, "c");
    assert!(!cleared.options.filter.enabled);
}

#[test]
fn tree_prefix_oversized_query_preserves_last_accepted_intent() {
    let mut p = fixture();
    let request = prefix_request(&mut p, "c");
    p.apply_with_sender(prefix_receipt(&request, &[1]), &|_| true);
    p.change_search_query(PaneId::Left, &"x".repeat(4097), &|_| panic!("over budget"));
    assert_eq!(p.panes[0].search.query, "c");
    assert_eq!(p.panes[0].search_match_count(), 1);
    assert!(p.panes[0].search_allows_activation());
}

#[test]
fn tree_prefix_context_caret_cannot_activate_with_empty_query() {
    let mut p = fixture();
    let original = p.panes[0].tree_view().unwrap();
    let mut rows = original.rows.to_vec();
    rows[0].context_only = true;
    rows[0].selectable = false;
    let mut stamp = original.stamp;
    stamp.revision = TreeRevision(2);
    let view = Arc::new(
        TreeProjection::new(original.root.clone(), stamp, TreeLayoutRevision(2), rows).unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(
            TreePublication::new(view, SelectionRevision(0), selection).unwrap(),
        ))),
        &|_| true,
    );
    p.panes[0].cursor = Some(EntryId(7));
    p.panes[0].parent_cursor = false;
    assert!(!p.panes[0].search_allows_activation());
}

#[test]
fn tree_prefix_retryable_mailbox_failure_retries_only_on_later_pump() {
    let mut p = fixture();
    let request = prefix_request(&mut p, "c");
    p.apply_with_sender(Event::Tree(Box::new(TreeEvent::ReprojectFailed {
        stamp: request.view.stamp,
        request: request.request,
        error: TreeDiagnostic::new("Queue full"),
        retryable: true,
    })), &|command| {
        assert!(!matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Reproject(_))),
            "same-basis queue pressure must not create a receipt loop");
        true
    });
    assert!(p.panes[0].search.pending);
    assert!(
        p.panes[0].error.is_empty(),
        "automatic search retry is not a panel failure"
    );
    let sent = RefCell::new(None);
    p.flush_search(&|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Reproject(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    assert_eq!(sent.into_inner().unwrap().options.search.query, "c");
}

#[test]
fn tree_prefix_escape_after_empty_filter_restores_last_usable_occurrence() {
    let mut p = fixture();
    p.preferences.values.quick_search_filter = true;
    let request = prefix_request(&mut p, "c");
    p.apply_with_sender(prefix_receipt(&request, &[1]), &|_| true);
    assert_eq!(p.panes[0].cursor, Some(EntryId(11)));
    let missing = prefix_request(&mut p, "zz");
    p.apply_with_sender(prefix_receipt(&missing, &[]), &|_| true);
    assert_eq!(p.panes[0].display_len(), 0);
    assert!(!p.panes[0].search_allows_activation());
    let cleared = prefix_request(&mut p, "");
    p.apply_with_sender(prefix_receipt(&cleared, &[]), &|_| true);
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(11)),
        "Escape restores the last focused raw occurrence"
    );
    assert!(p.panes[0].search_allows_activation());
}

#[test]
fn tree_prefix_failed_selection_releases_waiting_query_without_reusing_seed_revision() {
    let mut p = fixture();
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(11)),
        &|_| true
    ));
    p.change_search_query(PaneId::Left, "c", &|_| panic!("selection is pending"));
    let stamp = p.panes[0].tree_view().unwrap().stamp;
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionFailed {
            stamp,
            revision: SelectionRevision(2),
            error: TreeDiagnostic::new("Future selection"),
        })),
        &|_| panic!("a future failure cannot resolve a pending selection"),
    );
    assert_eq!(p.panes[0].tree.preview.len(), 1);
    let sent = RefCell::new(None);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionFailed {
            stamp,
            revision: SelectionRevision(1),
            error: TreeDiagnostic::new("Selection queue full"),
        })),
        &|command| {
            if let Command::Tree(command) = command
                && let TreeCommand::Reproject(request) = *command
            {
                *sent.borrow_mut() = Some(request);
            }
            true
        },
    );
    let request = sent
        .into_inner()
        .expect("failed selection releases waiting prefix");
    assert_eq!(request.selection_revision, SelectionRevision(0));
    assert_eq!(p.panes[0].tree.revision, SelectionRevision(1));
}

#[test]
fn tree_prefix_later_selection_rejection_waits_for_earlier_success() {
    let mut p = fixture();
    for row in [TreeRowId(11), TreeRowId(7)] {
        assert!(p.send_tree_selection(PaneId::Left, TreeSelectionAction::Toggle(row), &|_| true));
    }
    p.change_search_query(PaneId::Left, "c", &|_| panic!("selection is pending"));
    let published = p.panes[0].tree.publication.as_ref().unwrap().clone();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionFailed {
            stamp: published.view.stamp,
            revision: SelectionRevision(2),
            error: TreeDiagnostic::new("Second selection queue full"),
        })),
        &|_| panic!("the earlier selection still needs its receipt"),
    );
    assert_eq!(p.panes[0].tree.preview.len(), 1);
    let selection =
        Arc::new(TreeSelectionSnapshot::from_bits(published.view.clone(), vec![2]).unwrap());
    let sent = RefCell::new(None);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            view: published.view,
            revision: SelectionRevision(1),
            selection,
        })),
        &|command| {
            if let Command::Tree(command) = command
                && let TreeCommand::Reproject(request) = *command
            {
                *sent.borrow_mut() = Some(request);
            }
            true
        },
    );
    let request = sent
        .into_inner()
        .expect("all terminal selection receipts release prefix");
    assert_eq!(request.selection_revision, SelectionRevision(1));
    assert_eq!(request.selection.len(), 1);
    assert_eq!(p.panes[0].tree.revision, SelectionRevision(2));
}

#[test]
fn tree_prefix_insert_waits_for_clear_then_toggles_captured_match_once_and_advances() {
    let mut p = fixture();
    let query = prefix_request(&mut p, "f");
    p.apply_with_sender(prefix_receipt(&query, &[0]), &|_| true);
    let clear = prefix_request(&mut p, "");
    p.move_cursor(PaneId::Left, 9, &|_| {
        panic!("Insert waits for empty projection Commit")
    });
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(prefix_receipt(&clear, &[]), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(sent.borrow().iter().any(|command| matches!(command,
        Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Select(request)
            if matches!(request.action, TreeSelectionAction::Toggle(TreeRowId(7)))))));
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(11)),
        "Insert advances after toggle admission"
    );
    p.apply_with_sender(prefix_receipt(&clear, &[]), &|command| {
        assert!(!matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Select(_))), "duplicate receipt cannot toggle twice");
        true
    });
}

#[test]
fn tree_prefix_insert_is_retired_by_new_query_navigation_or_zero_matches() {
    for action in ["typing", "navigation", "zero"] {
        let mut p = fixture();
        let query = prefix_request(&mut p, if action == "zero" { "zz" } else { "c" });
        p.apply_with_sender(
            prefix_receipt(&query, if action == "zero" { &[] } else { &[1] }),
            &|_| true,
        );
        let clear = prefix_request(&mut p, "");
        p.move_cursor(PaneId::Left, 9, &|_| {
            panic!("Insert waits for clear receipt")
        });
        match action {
            "typing" => p.change_search_query(PaneId::Left, "f", &|_| true),
            "navigation" => {
                let path = p.panes[0].path.join("elsewhere");
                assert!(p.navigate(PaneId::Left, path, None, &|_| true));
            }
            _ => {}
        }
        p.apply_with_sender(prefix_receipt(&clear, &[]), &|command| {
            assert!(!matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Select(_))), "obsolete or zero-result Insert cannot select");
            true
        });
    }
}

#[test]
fn tree_prefix_insert_is_retired_when_branch_hides_its_captured_occurrence() {
    let mut p = fixture();
    let query = prefix_request(&mut p, "c");
    p.apply_with_sender(prefix_receipt(&query, &[1]), &|_| true);
    let clear = prefix_request(&mut p, "");
    p.move_cursor(PaneId::Left, 9, &|_| {
        panic!("Insert waits for clear receipt")
    });
    let mut rows = clear.view.rows.to_vec();
    rows.truncate(1);
    rows[0].expanded = false;
    rows[0].disclosure = TreeDisclosure::Collapsed;
    let mut stamp = clear.view.stamp;
    stamp.revision = TreeRevision(4);
    let view = Arc::new(
        TreeProjection::new(clear.view.root.clone(), stamp, TreeLayoutRevision(4), rows).unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(
            TreePublication::new(view, SelectionRevision(0), selection).unwrap(),
        ))),
        &|_| true,
    );
    let retry = RefCell::new(None);
    p.apply_with_sender(prefix_receipt(&clear, &[]), &|command| {
        if let Command::Tree(command) = command {
            match *command {
                TreeCommand::Select(_) => panic!("hidden occurrence cannot be toggled"),
                TreeCommand::Reproject(request) => *retry.borrow_mut() = Some(request),
                _ => {}
            }
        }
        true
    });
    let retry = retry.into_inner().unwrap();
    p.apply_with_sender(prefix_receipt(&retry, &[]), &|command| {
        assert!(!matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Select(_))), "a later row cannot resurrect the retired Insert");
        true
    });
}

#[test]
fn tree_prefix_stale_hidden_row_publication_cannot_cancel_captured_insert() {
    let mut p = fixture();
    let query = prefix_request(&mut p, "c");
    p.apply_with_sender(prefix_receipt(&query, &[1]), &|_| true);
    let clear = prefix_request(&mut p, "");
    p.move_cursor(PaneId::Left, 9, &|_| {
        panic!("Insert waits for clear receipt")
    });
    let mut rows = clear.view.rows.to_vec();
    rows.truncate(1);
    rows[0].expanded = false;
    rows[0].disclosure = TreeDisclosure::Collapsed;
    let mut stamp = clear.view.stamp;
    stamp.revision = TreeRevision(1);
    let view = Arc::new(
        TreeProjection::new(clear.view.root.clone(), stamp, TreeLayoutRevision(1), rows).unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(
            TreePublication::new(view, SelectionRevision(0), selection).unwrap(),
        ))),
        &|_| true,
    );
    let selected = RefCell::new(None);
    p.apply_with_sender(prefix_receipt(&clear, &[]), &|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Select(request) = *command
        {
            *selected.borrow_mut() = Some(request.action);
        }
        true
    });
    assert!(
        matches!(
            selected.into_inner(),
            Some(TreeSelectionAction::Toggle(TreeRowId(11)))
        ),
        "a rejected old publication cannot retire a current Insert"
    );
}

#[test]
fn tree_prefix_insert_cannot_toggle_old_caret_when_query_never_reached_backend() {
    let mut p = fixture();
    p.panes[0].cursor = Some(EntryId(7));
    p.panes[0].parent_cursor = false;
    p.change_search_query(PaneId::Left, "c", &|_| false);
    p.change_search_query(PaneId::Left, "", &|_| true);
    p.move_cursor(PaneId::Left, 9, &|command| {
        assert!(!matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Select(_))),
            "unaccepted query has no valid matching Insert target");
        true
    });
    p.flush_search(&|_| true);
    p.flush_selection(&|command| {
        assert!(!matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Select(_))),
            "an invalid Insert must not survive the next pump");
        true
    });
    assert!(!p.panes[0].search.pending);
}

#[test]
fn tree_prefix_insert_backpressure_retains_one_exact_target_across_caret_movement() {
    let mut p = fixture();
    let query = prefix_request(&mut p, "f");
    p.apply_with_sender(prefix_receipt(&query, &[0]), &|_| true);
    let clear = prefix_request(&mut p, "");
    p.move_cursor(PaneId::Left, 9, &|_| {
        panic!("Insert waits for clear receipt")
    });
    p.apply_with_sender(prefix_receipt(&clear, &[]), &|command| {
        !matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Select(_)))
    });
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(7)),
        "refused toggle cannot advance caret"
    );
    p.focus_display_row(PaneId::Left, 2);
    assert_eq!(p.panes[0].cursor, Some(EntryId(11)));
    p.move_cursor(PaneId::Left, 9, &|_| {
        panic!("repeat Insert cannot duplicate its pending gesture")
    });
    let selected = RefCell::new(Vec::new());
    p.flush_selection(&|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Select(request) = *command
        {
            selected.borrow_mut().push(request.action);
        }
        true
    });
    assert_eq!(selected.borrow().len(), 1);
    assert!(
        matches!(
            selected.borrow()[0],
            TreeSelectionAction::Toggle(TreeRowId(7))
        ),
        "pending Insert never substitutes a later caret"
    );
}

fn prefix_receipt_with_second_child_match(request: &TreeReprojectRequest) -> Event {
    let Event::Tree(mut event) = prefix_receipt(request, &[1]) else {
        unreachable!();
    };
    let TreeEvent::Reprojected { publication, .. } = event.as_mut() else {
        unreachable!();
    };
    let mut rows = publication.view.rows.to_vec();
    let mut second = rows[1].clone();
    second.id = TreeRowId(13);
    second.entry = Entry::new(
        EntryId(13),
        rows[0].entry.path.join("child-second.txt"),
        EntryKind::File,
    );
    rows.push(second);
    let view = Arc::new(
        TreeProjection::new(
            publication.view.root.clone(),
            publication.view.stamp,
            publication.view.layout_revision,
            rows,
        )
        .unwrap(),
    );
    let selection = Arc::new(request.selection.rebase(view.clone()).unwrap());
    *publication = TreePublication::new(view, publication.selection_revision, selection).unwrap();
    Event::Tree(event)
}

#[test]
fn tree_prefix_filter_preference_keeps_matching_caret_and_replaces_hidden_caret() {
    for (matching_caret, expected) in [(false, EntryId(11)), (true, EntryId(13))] {
        let mut p = fixture();
        let request = prefix_request(&mut p, "c");
        p.apply_with_sender(prefix_receipt_with_second_child_match(&request), &|_| true);
        p.focus_display_row(PaneId::Left, if matching_caret { 3 } else { 1 });
        p.begin_options();
        let mut preferences = p.preferences.values.clone();
        preferences.quick_search_filter = true;
        p.update_preferences(preferences, &|_| true);
        let sent = RefCell::new(None);
        p.flush_search(&|command| {
            if let Command::Tree(command) = command
                && let TreeCommand::Reproject(request) = *command
            {
                *sent.borrow_mut() = Some(request);
            }
            true
        });
        let filtered = sent.into_inner().unwrap();
        p.apply_with_sender(prefix_receipt_with_second_child_match(&filtered), &|_| true);
        assert_eq!(
            p.panes[0].cursor,
            Some(expected),
            "quick-filter preserves a matching caret and moves a hidden caret to its first match"
        );
        assert!(p.panes[0].search_allows_activation());
    }
}

#[test]
fn tree_filter_history_waits_for_successful_correlated_publication() {
    let mut p = fixture();
    let request = enqueue(&mut p);
    assert!(p.preferences.values.panel_filter_history.is_empty());
    p.apply_with_sender(receipt(&request), &|_| true);
    assert_eq!(
        p.preferences.values.panel_filter_history,
        vec![request.options.filter]
    );
}
