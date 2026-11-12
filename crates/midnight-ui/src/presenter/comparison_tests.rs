use super::*;
use std::cell::RefCell;

fn loaded() -> Presenter {
    let mut p = Presenter::new("left".into(), "right".into());
    for (slot, pane) in p.panes.iter_mut().enumerate() {
        pane.generation = 1;
        pane.successful_generation = Some(NavigationId(1));
        pane.selection_context = NavigationId(1);
        pane.selection_committed_context = NavigationId(1);
        pane.entries = vec![Entry::new(
            EntryId(1),
            pane.path.join("one"),
            EntryKind::File,
        )]
        .into();
        pane.index = Arc::new(DirectoryIndex::from_entries(&pane.entries));
        pane.cursor = Some(EntryId(1));
        assert_eq!(slot, if pane.path == Path::new("left") { 0 } else { 1 });
    }
    p
}

fn start(p: &mut Presenter) -> Arc<CompareRequest> {
    let commands = RefCell::new(Vec::new());
    p.open_comparison();
    p.submit_comparison(CompareOptions::default(), &|command| {
        commands.borrow_mut().push(command);
        true
    });
    commands
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::CompareDirectories(request) => Some(Arc::new(request)),
            _ => None,
        })
        .unwrap()
}

fn result(request: Arc<CompareRequest>) -> Event {
    let selections = request
        .sides
        .each_ref()
        .map(|side| Arc::new(SelectionSnapshot::from_bits(side.view.clone(), vec![1]).unwrap()));
    Event::DirectoriesCompared {
        id: request.id,
        result: Ok(Arc::new(CompareResult {
            request,
            selections,
            tree_replacements: [None, None],
            unknown: 0,
            ambiguous: 0,
        })),
    }
}

#[test]
fn comparison_publishes_both_and_next_toggle_carries_authoritative_seed() {
    let mut p = loaded();
    let request = start(&mut p);
    p.apply(result(request));
    assert_eq!(p.panes.each_ref().map(|pane| pane.selected.len()), [1, 1]);
    let commands = RefCell::new(Vec::new());
    p.send_selection(
        PaneId::Left,
        SelectionAction::Toggle(Path::new("left/one").into()),
        &|command| {
            commands.borrow_mut().push(command);
            true
        },
    );
    let Command::Selection(request) = commands.into_inner().pop().unwrap() else {
        panic!("selection");
    };
    assert_eq!(request.seed_revision, SelectionRevision(1));
    assert_eq!(request.revision, SelectionRevision(2));
    assert_eq!(request.seed.len(), 1);
    let authority = request.authoritative.as_ref().unwrap();
    assert_eq!(authority.id, 1);
    assert_eq!(authority.origin.navigation, NavigationId(1));
    assert_eq!(authority.snapshot.len(), 1);
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        revision: request.revision,
        selection: Arc::new(SelectionSnapshot::from_bits(request.view.unwrap(), vec![0]).unwrap()),
        deferred: Default::default(),
    });
    assert!(
        p.comparison.seeds[0].is_none(),
        "acknowledged source index must not remain pinned by the seed"
    );
}

#[test]
fn comparison_stamp_is_carried_across_navigation_revision_reset() {
    let mut p = loaded();
    let request = start(&mut p);
    p.apply(result(request));
    p.selection_navigation_accepted(PaneId::Left, NavigationId(2), Path::new("left"));
    p.panes[0].generation = 2;
    let commands = RefCell::new(Vec::new());
    p.send_selection(PaneId::Left, SelectionAction::Rebase, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    let Command::Selection(request) = commands.into_inner().pop().unwrap() else {
        panic!("selection");
    };
    assert_eq!(request.navigation, NavigationId(2));
    assert_eq!(request.seed_revision, SelectionRevision(0));
    let authority = request.authoritative.unwrap();
    assert_eq!(authority.origin.navigation, NavigationId(1));
    assert_eq!(authority.snapshot.len(), 1);
}

#[test]
fn selection_edit_navigation_or_cancel_rejects_both_comparison_results() {
    for change in 0..3 {
        let mut p = loaded();
        let request = start(&mut p);
        match change {
            0 => p.panes[1].selection_revision.0 += 1,
            1 => p.panes[0].successful_generation = Some(NavigationId(2)),
            _ => p.cancel_comparison(&|_| true),
        }
        p.apply(result(request));
        assert!(p.panes.iter().all(|pane| pane.selected.is_empty()));
    }
}

#[test]
fn invalid_or_cancelled_filter_validation_cannot_change_saved_filters() {
    let mut p = loaded();
    p.open_panel_filter(PaneId::Left);
    let commands = RefCell::new(Vec::new());
    let filter = PanelFilter {
        enabled: true,
        expression: "*.txt".into(),
        ..Default::default()
    };
    p.submit_panel_filter(PaneId::Left, filter.clone(), &|command| {
        commands.borrow_mut().push(command);
        true
    });
    let Command::ValidatePanelFilter(request) = commands.into_inner().pop().unwrap() else {
        panic!("validation");
    };
    p.cancel_panel_filter();
    p.apply(Event::PanelFilterValidated {
        request,
        result: Ok(()),
        diagnostic: None,
    });
    p.flush_filters(&|_| true);
    assert!(!p.preferences.values.panel_filters[0].enabled);
    p.open_panel_filter(PaneId::Left);
    let commands = RefCell::new(Vec::new());
    p.submit_panel_filter(PaneId::Left, filter, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    let Command::ValidatePanelFilter(request) = commands.into_inner().pop().unwrap() else {
        panic!("validation");
    };
    p.apply(Event::PanelFilterValidated {
        request,
        result: Ok(()),
        diagnostic: None,
    });
    p.flush_filters(&|_| true);
    assert!(p.preferences.values.panel_filters[0].enabled);
    assert!(!p.preferences.values.panel_filters[1].enabled);
    assert!(p.panes[0].search_view_waiting());
    assert!(!p.panes[0].search_allows_activation());
}

#[test]
fn persistent_empty_projection_cannot_activate_a_previously_visible_cursor() {
    for (mode, expression) in [
        (FilterMode::Masks, "*.none"),
        (FilterMode::Expression, "kind = file and size > 100GiB"),
    ] {
        let mut p = loaded();
        p.preferences.values.panel_filters[0] = PanelFilter {
            enabled: true,
            mode,
            expression: expression.into(),
            ..Default::default()
        };
        p.search_filter_changed();
        let commands = RefCell::new(Vec::new());
        p.flush_search(&|command| {
            commands.borrow_mut().push(command);
            true
        });
        let request = commands
            .into_inner()
            .into_iter()
            .find_map(|command| match command {
                Command::Search(request) => Some(request),
                _ => None,
            })
            .unwrap();
        p.apply(Event::SearchReady {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            revision: request.revision,
            result: Arc::new(SearchResult {
                source: request.source,
                matches: Arc::default(),
                index: Arc::default(),
                filtered: Arc::default(),
                filtered_index: Arc::default(),
            }),
        });
        assert_eq!(p.panes[0].display_len(), 0);
        assert!(
            !p.panes[0].search_allows_activation(),
            "hidden cursor must not become a fallback operation target"
        );
    }
}
