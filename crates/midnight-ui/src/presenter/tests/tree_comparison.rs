use super::*;

#[test]
fn directory_comparison_remains_available_after_inline_tree_adoption() {
    let (mut p, root) = super::tree::fixture();
    let publication = super::tree::publication(root, 1, true, false);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    p.open_comparison();
    assert!(
        p.comparison.open,
        "expanding a native branch must not disable directory comparison"
    );
}

fn loaded_comparison() -> Presenter {
    let (mut p, root) = super::tree::fixture();
    let publication = super::tree::publication(root, 1, true, false);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    let right = test_root().join("comparison-right");
    let navigation = begin(&mut p, PaneId::Right, right.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Right,
        navigation,
        vec![Entry::new(
            EntryId(2),
            right.join("only-right.txt"),
            EntryKind::File,
        )],
    );
    p
}

fn start_tree_comparison() -> (Presenter, Arc<CompareRequest>) {
    let mut p = loaded_comparison();
    let commands = RefCell::new(Vec::new());
    p.open_comparison();
    p.submit_comparison(CompareOptions::default(), &|command| {
        commands.borrow_mut().push(command);
        true
    });
    let request = commands
        .into_inner()
        .into_iter()
        .find_map(|command| {
            if let Command::CompareDirectories(request) = command {
                Some(Arc::new(request))
            } else {
                None
            }
        })
        .expect("comparison worker request");
    assert!(request.sides[0].tree.is_some());
    assert!(request.sides[1].tree.is_none());
    (p, request)
}

fn compared(request: Arc<CompareRequest>) -> Event {
    let left =
        Arc::new(SelectionSnapshot::from_bits(request.sides[0].view.clone(), vec![1]).unwrap());
    let right =
        Arc::new(SelectionSnapshot::from_bits(request.sides[1].view.clone(), vec![1]).unwrap());
    let tree = Arc::new(
        TreeSelectionSnapshot::from_bits(request.sides[0].tree.clone().unwrap(), vec![1]).unwrap(),
    );
    Event::DirectoriesCompared {
        id: request.id,
        result: Ok(Arc::new(CompareResult {
            request,
            selections: [left, right],
            tree_replacements: [
                Some(Arc::new(
                    TreeSelectionReplacement::from_snapshot(&tree).unwrap(),
                )),
                None,
            ],
            unknown: 0,
            ambiguous: 0,
        })),
    }
}

#[test]
fn tree_comparison_selection_is_immediate_and_waits_for_worker_acknowledgement() {
    let (mut p, request) = start_tree_comparison();
    p.apply(compared(request));
    let view = p.panes[0].tree_view().cloned().unwrap();
    assert_eq!(
        p.panes[0].tree_selection_visible(&view.rows[0].entry),
        Some(true)
    );
    assert_eq!(
        p.panes[0].tree_selection_visible(&view.rows[1].entry),
        Some(false)
    );
    assert_eq!(p.panes[1].selected.len(), 1);
    assert!(
        p.comparison.busy,
        "pending backend selection is not reported as finished"
    );
    let commands = RefCell::new(Vec::new());
    p.flush_selection(&|command| {
        commands.borrow_mut().push(command);
        true
    });
    let selection = commands
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Tree(command) => match *command {
                TreeCommand::Select(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("typed tree selection request");
    assert!(matches!(selection.action, TreeSelectionAction::Replace(_)));
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            view: view.clone(),
            revision: selection.revision,
            selection: Arc::new(TreeSelectionSnapshot::from_bits(view, vec![1]).unwrap()),
        })),
        &|_| true,
    );
    assert!(!p.comparison.busy);
    assert!(p.comparison.error.is_empty());
}

#[test]
fn changed_tree_or_selection_discards_comparison_before_either_side_is_updated() {
    for change in 0..2 {
        let (mut p, request) = start_tree_comparison();
        if change == 0 {
            p.panes[0].tree.revision.0 += 1;
        } else {
            p.panes[1].generation += 1;
            p.panes[1].successful_generation = None;
        }
        p.apply(compared(request));
        assert!(p.panes[0].tree.preview.is_empty());
        assert!(p.panes[1].selected.is_empty());
        assert!(!p.comparison.error.is_empty());
    }
}

#[test]
fn rejected_tree_comparison_selection_reports_failure_without_success_claim() {
    let (mut p, request) = start_tree_comparison();
    p.apply(compared(request));
    let stamp = p.panes[0].tree_view().unwrap().stamp;
    let revision = p.panes[0].tree.revision;
    p.flush_selection(&|_| true);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionFailed {
            stamp,
            revision,
            error: TreeDiagnostic::new("stale capture"),
        })),
        &|_| true,
    );
    assert!(!p.comparison.busy);
    assert!(p.comparison.error.contains("Compare again"));
    let commands = RefCell::new(Vec::new());
    p.submit_comparison(CompareOptions::default(), &|command| {
        commands.borrow_mut().push(command);
        true
    });
    assert!(
        commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::CompareDirectories(_))),
        "a rejected selection must allow an immediate comparison retry"
    );
}

#[test]
fn deselect_all_clears_marked_root_and_nested_folders_immediately() {
    let (mut p, root) = super::tree::fixture();
    let mut publication = super::tree::publication(root, 1, true, true);
    publication.selection =
        Arc::new(TreeSelectionSnapshot::from_bits(publication.view.clone(), vec![3]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    let view = p.panes[0].tree_view().cloned().unwrap();
    assert!(
        p.bulk_selection(PaneId::Left, SelectionOperation::Clear, &|command| {
            let Command::Tree(command) = command else {
                panic!("expected a typed tree command");
            };
            assert!(matches!(
                *command,
                TreeCommand::Select(TreeSelectionRequest {
                    action: TreeSelectionAction::Bulk(SelectionOperation::Clear),
                    ..
                })
            ));
            true
        })
    );
    for row in view.rows.iter() {
        assert_eq!(p.panes[0].tree_selection_visible(&row.entry), Some(false));
    }
}

#[test]
fn waiting_comparison_dispatches_on_the_last_tree_selection_receipt() {
    let mut p = loaded_comparison();
    assert!(p.bulk_selection(PaneId::Left, SelectionOperation::All, &|_| true));
    let view = p.panes[0].tree_view().cloned().unwrap();
    let revision = p.panes[0].tree.revision;
    p.submit_comparison(CompareOptions::default(), &|command| {
        assert!(!matches!(command, Command::CompareDirectories(_)));
        true
    });
    assert!(p.comparison.busy);
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            selection: Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap()),
            view,
            revision,
        })),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    let commands = sent.borrow();
    let comparisons = commands
        .iter()
        .filter_map(|command| match command {
            Command::CompareDirectories(request) => Some(request),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        comparisons.len(),
        1,
        "the final tree receipt must unblock comparison"
    );
    assert_eq!(comparisons[0].sides[0].selection_revision, revision);
}

fn reprojected(request: TreeReprojectRequest) -> Event {
    Event::Tree(Box::new(TreeEvent::Reprojected {
        previous: request.view.stamp,
        request: request.request,
        options: request.options,
        publication: super::tree::publication(
            request.view.root.clone(),
            request.view.stamp.revision.0 + 1,
            true,
            false,
        ),
    }))
}

#[test]
fn waiting_comparison_dispatches_when_tree_search_clear_is_acknowledged() {
    let mut p = loaded_comparison();
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.change_search_query(PaneId::Left, "folder", &send);
    let request = sent
        .borrow_mut()
        .drain(..)
        .find_map(|command| match command {
            Command::Tree(command) => match *command {
                TreeCommand::Reproject(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("tree search request");
    p.apply_with_sender(reprojected(request), &|_| true);
    p.submit_comparison(CompareOptions::default(), &send);
    let commands = sent.replace(Vec::new());
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::CompareDirectories(_)))
    );
    let clear = commands
        .into_iter()
        .find_map(|command| match command {
            Command::Tree(command) => match *command {
                TreeCommand::Reproject(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("comparison clears tree search first");
    assert!(clear.options.search.query.is_empty());
    p.apply_with_sender(reprojected(clear), &send);
    assert_eq!(
        sent.borrow()
            .iter()
            .filter(|command| matches!(command, Command::CompareDirectories(_)))
            .count(),
        1
    );
}

#[test]
fn new_tree_publication_cancels_an_inflight_comparison_without_unrelated_events() {
    let (mut p, request) = start_tree_comparison();
    let view = request.sides[0].tree.as_ref().unwrap();
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(super::tree::publication(
            view.root.clone(),
            view.stamp.revision.0 + 1,
            true,
            false,
        )))),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(!p.comparison.busy);
    assert!(p.comparison.error.contains("cancelled"));
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::CancelComparison(id) if *id == request.id))
    );
}
