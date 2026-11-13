use super::*;

fn flat_fixture(kind: EntryKind) -> (Presenter, TreeRootSnapshot) {
    let path = test_root().join("tree");
    let mut p = Presenter::new(path.clone(), path.clone());
    let navigation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(42), path.join("folder"), kind),
            Entry::new(EntryId(11), path.join("child"), EntryKind::File),
        ],
    );
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation,
        entries: p.panes[0].entries.clone(),
        index: p.panes[0].index.clone(),
    };
    (p, root)
}

pub(super) fn fixture() -> (Presenter, TreeRootSnapshot) {
    let (mut p, root) = flat_fixture(EntryKind::Directory);
    let identity = p.panes[0].row_identity(&root.entries[0]);
    p.disclose_tree(PaneId::Left, &identity, &|_| true);
    (p, root)
}

pub(super) fn publication(
    root: TreeRootSnapshot,
    revision: u64,
    child: bool,
    selected: bool,
) -> TreePublication {
    let mut entry = root.entries[0].clone();
    entry.id = EntryId(7);
    let mut rows = vec![TreeDisplayRow {
        id: TreeRowId(7),
        entry,
        parent: None,
        depth: 0,
        disclosure: if child {
            TreeDisclosure::Expanded
        } else {
            TreeDisclosure::Collapsed
        },
        expanded: child,
        selectable: true,
        navigable: true,
        context_only: false,
        error: None,
        error_details: None,
        original_root_entry: Some(EntryId(42)),
        prefix_end: 0,
        search_match: false,
    }];
    if child {
        rows.push(TreeDisplayRow {
            id: TreeRowId(11),
            entry: Entry::new(
                EntryId(11),
                root.entries[0].path.join("child"),
                EntryKind::Directory,
            ),
            parent: Some(TreeRowId(7)),
            depth: 1,
            disclosure: TreeDisclosure::Unknown,
            expanded: false,
            selectable: true,
            navigable: true,
            context_only: false,
            error: None,
            error_details: None,
            original_root_entry: None,
            prefix_end: 0,
            search_match: false,
        });
    }
    let stamp = TreeViewStamp {
        pane: root.pane,
        navigation: root.navigation,
        session: TreeSessionId(5),
        revision: TreeRevision(revision),
    };
    let view =
        Arc::new(TreeProjection::new(root, stamp, TreeLayoutRevision(revision), rows).unwrap());
    let selection = Arc::new(
        TreeSelectionSnapshot::from_bits(view.clone(), vec![if selected { 1 } else { 0 }]).unwrap(),
    );
    TreePublication::new(view, SelectionRevision(0), selection).unwrap()
}

#[test]
fn recursive_disclosure_retains_action_through_enable_commit_and_branch_backpressure() {
    for kind in [EntryKind::Directory, EntryKind::DirectoryLink] {
        let (mut p, root) = flat_fixture(kind);
        let identity = p.panes[0].row_identity(&root.entries[0]);
        let original_selection = p.panes[0].selected.clone();
        let sent = RefCell::new(Vec::new());
        p.disclose_tree_recursive(PaneId::Left, &identity, &|command| {
            sent.borrow_mut().push(command);
            false
        });
        assert!(matches!(&sent.borrow()[0], Command::Tree(command)
            if matches!(command.as_ref(), TreeCommand::Enable { root: offered, .. }
                if offered.same_snapshot(&root))));
        assert!(Arc::ptr_eq(&p.panes[0].selected, &original_selection));
        sent.borrow_mut().clear();
        p.flush_trees(&|command| {
            sent.borrow_mut().push(command);
            true
        });
        assert!(matches!(&sent.borrow()[0], Command::Tree(command)
            if matches!(command.as_ref(), TreeCommand::Enable { root: offered, .. }
                if offered.same_snapshot(&root))));

        let mut foreign = root.clone();
        foreign.index = Arc::new((*foreign.index).clone());
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(publication(
                foreign, 1, false, false,
            )))),
            &|_| panic!("a foreign publication cannot commit or expand"),
        );
        assert!(p.panes[0].tree_view().is_none());
        sent.borrow_mut().clear();
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(publication(
                root, 1, false, false,
            )))),
            &|command| {
                sent.borrow_mut().push(command);
                false
            },
        );
        assert!(sent.borrow().iter().any(|command| matches!(command,
            Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Commit(_)))));
        assert!(!sent.borrow().iter().any(|command| matches!(command,
            Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Branch { .. }))));
        p.disclose_tree_recursive(PaneId::Left, &identity, &|_| {
            panic!("a retired flat row cannot retarget the adopted tree")
        });

        let view = p.panes[0].tree_view().unwrap().clone();
        sent.borrow_mut().clear();
        p.flush_trees(&|command| {
            let accepted = !matches!(&command,
                Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Branch { .. }));
            sent.borrow_mut().push(command);
            accepted
        });
        let sent_commands = sent.borrow();
        assert!(matches!(&sent_commands[0], Command::Tree(command)
            if matches!(command.as_ref(), TreeCommand::Commit(committed)
                if Arc::ptr_eq(committed, &view))));
        assert!(matches!(&sent_commands[1], Command::Tree(command)
            if matches!(command.as_ref(), TreeCommand::Branch {
                view: requested, row: TreeRowId(7), action: TreeBranchAction::ExpandRecursive, ..
            } if Arc::ptr_eq(requested, &view))));
        assert!(!sent_commands.iter().any(|command| matches!(command,
            Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Select(_)))));
        drop(sent_commands);
        sent.borrow_mut().clear();
        p.flush_trees(&|command| {
            sent.borrow_mut().push(command);
            true
        });
        assert_eq!(sent.borrow().len(), 1);
        assert!(matches!(&sent.borrow()[0], Command::Tree(command)
            if matches!(command.as_ref(), TreeCommand::Branch {
                view: requested, row: TreeRowId(7), action: TreeBranchAction::ExpandRecursive, ..
            } if Arc::ptr_eq(requested, &view))));
        assert_eq!(p.panes[0].selected_count(), 0);
        sent.borrow_mut().clear();
        p.flush_trees(&|command| {
            sent.borrow_mut().push(command);
            true
        });
        assert!(
            sent.borrow().is_empty(),
            "accepted recursive intent is sent once"
        );
    }
}

#[test]
fn recursive_disclosure_expands_open_rows_while_ordinary_disclosure_collapses() {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root.clone(),
            1,
            true,
            false,
        )))),
        &|_| true,
    );
    let view = p.panes[0].tree_view().unwrap().clone();
    let identity = p.panes[0].row_identity(&view.rows[0].entry);
    let cursor = p.panes[0].cursor;
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.disclose_tree_recursive(PaneId::Left, &identity, &send);
    p.disclose_tree(PaneId::Left, &identity, &send);
    let commands = sent.borrow();
    assert_eq!(commands.len(), 2);
    assert!(matches!(&commands[0], Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Branch {
            view: requested, row: TreeRowId(7), action: TreeBranchAction::ExpandRecursive, ..
        } if Arc::ptr_eq(requested, &view))));
    assert!(matches!(&commands[1], Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Branch {
            view: requested, row: TreeRowId(7), action: TreeBranchAction::Collapse, ..
        } if Arc::ptr_eq(requested, &view))));
    assert_eq!(p.panes[0].cursor, cursor);
    assert_eq!(p.panes[0].selected_count(), 0);
    drop(commands);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 2, true, false,
        )))),
        &|_| true,
    );
    p.disclose_tree_recursive(PaneId::Left, &identity, &|_| {
        panic!("a stale row identity cannot recursively expand a newer publication")
    });
}

#[test]
fn recursive_disclosure_intent_is_retired_by_navigation_before_commit() {
    let (mut p, root) = flat_fixture(EntryKind::Directory);
    let identity = p.panes[0].row_identity(&root.entries[0]);
    p.disclose_tree_recursive(PaneId::Left, &identity, &|_| true);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root.clone(),
            1,
            false,
            false,
        )))),
        &|_| false,
    );
    begin(
        &mut p,
        PaneId::Left,
        test_root().join("elsewhere").to_str().unwrap(),
    );
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 2, false, false,
        )))),
        &|command| {
            assert!(
                !matches!(command, Command::Tree(_)),
                "retired recursive intent cannot send"
            );
            true
        },
    );
    assert!(p.panes[0].tree_view().is_none());
}

#[test]
fn accepted_ordinary_disclosure_supersedes_a_queued_recursive_intent_for_the_same_row() {
    for (expanded, ordinary) in [
        (false, TreeBranchAction::Expand),
        (true, TreeBranchAction::Collapse),
    ] {
        let (mut p, root) = flat_fixture(EntryKind::Directory);
        let identity = p.panes[0].row_identity(&root.entries[0]);
        p.disclose_tree_recursive(PaneId::Left, &identity, &|_| true);
        let sent = RefCell::new(Vec::new());
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(publication(root, 1, expanded, false)))),
            &|command| {
                let accepted = !matches!(&command,
                    Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Branch { .. }));
                sent.borrow_mut().push(command);
                accepted
            },
        );
        assert!(sent.borrow().iter().any(|command| matches!(command,
        Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Branch {
            row: TreeRowId(7), action: TreeBranchAction::ExpandRecursive, ..
        }))));
        let view = p.panes[0].tree_view().unwrap().clone();
        let identity = p.panes[0].row_identity(&view.rows[0].entry);
        p.disclose_tree(PaneId::Left, &identity, &|_| false);
        sent.borrow_mut().clear();
        p.flush_trees(&|command| {
            sent.borrow_mut().push(command);
            false
        });
        assert!(
            matches!(&sent.borrow()[0], Command::Tree(command)
            if matches!(command.as_ref(), TreeCommand::Branch {
                view: requested, row: TreeRowId(7), action: TreeBranchAction::ExpandRecursive, ..
            } if Arc::ptr_eq(requested, &view))),
            "a rejected newer gesture leaves the older recursive intent available"
        );

        sent.borrow_mut().clear();
        p.disclose_tree(PaneId::Left, &identity, &|command| {
            sent.borrow_mut().push(command);
            true
        });
        assert_eq!(sent.borrow().len(), 1);
        assert!(matches!(&sent.borrow()[0], Command::Tree(command)
            if matches!(command.as_ref(), TreeCommand::Branch {
                view: requested, row: TreeRowId(7), action, ..
            } if Arc::ptr_eq(requested, &view) && *action == ordinary)));
        sent.borrow_mut().clear();
        p.flush_trees(&|command| {
            sent.borrow_mut().push(command);
            true
        });
        assert!(
            !sent.borrow().iter().any(|command| matches!(command,
            Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Branch { .. }))),
            "an accepted newer gesture must not be overridden by the older recursive retry"
        );
        assert_eq!(p.panes[0].selected_count(), 0);
    }
}

#[test]
fn accepted_recursive_disclosure_of_another_row_supersedes_the_older_queued_start() {
    let (mut p, root) = flat_fixture(EntryKind::Directory);
    let first = p.panes[0].row_identity(&root.entries[0]);
    p.disclose_tree_recursive(PaneId::Left, &first, &|_| true);
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, true, false,
        )))),
        &|command| {
            let accepted = !matches!(&command,
                Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Branch { .. }));
            sent.borrow_mut().push(command);
            accepted
        },
    );
    assert!(sent.borrow().iter().any(|command| matches!(command,
    Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Branch {
        row: TreeRowId(7), action: TreeBranchAction::ExpandRecursive, ..
    }))));
    let view = p.panes[0].tree_view().unwrap().clone();
    assert_eq!(view.rows[0].entry.kind, EntryKind::Directory);
    assert_eq!(view.rows[1].entry.kind, EntryKind::Directory);
    let second = p.panes[0].row_identity(&view.rows[1].entry);
    p.disclose_tree_recursive(PaneId::Left, &second, &|_| false);
    sent.borrow_mut().clear();
    p.flush_trees(&|command| {
        sent.borrow_mut().push(command);
        false
    });
    assert!(
        matches!(&sent.borrow()[0], Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Branch {
            view: requested, row: TreeRowId(7), action: TreeBranchAction::ExpandRecursive, ..
        } if Arc::ptr_eq(requested, &view))),
        "a rejected recursive start on B leaves the queued start on A intact"
    );

    sent.borrow_mut().clear();
    p.disclose_tree_recursive(PaneId::Left, &second, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert_eq!(sent.borrow().len(), 1);
    assert!(matches!(&sent.borrow()[0], Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Branch {
            view: requested, row: TreeRowId(11), action: TreeBranchAction::ExpandRecursive, ..
        } if Arc::ptr_eq(requested, &view))));
    sent.borrow_mut().clear();
    p.flush_trees(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(
        !sent.borrow().iter().any(|command| matches!(command,
        Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Branch { .. }))),
        "the older start on A cannot replace the accepted recursive operation on B"
    );
    assert_eq!(p.panes[0].selected_count(), 0);
}

#[test]
fn tree_commit_backpressure_does_not_send_selection_before_acknowledgment() {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, false, false,
        )))),
        &|_| false,
    );
    let sent = RefCell::new(Vec::new());
    p.select_row_modified(PaneId::Left, 1, true, false, &|c| {
        sent.borrow_mut().push(c);
        true
    });
    assert!(
        sent.borrow().is_empty(),
        "selection cannot precede the exact Commit"
    );
    p.flush_selection(&|c| {
        sent.borrow_mut().push(c);
        true
    });
    assert!(
        matches!(&sent.borrow()[0], Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Commit(_)))
    );
}

#[test]
fn tree_folder_sizes_reaches_backend_after_exact_publication_commit() {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, true, false,
        )))),
        &|_| true,
    );
    let sent = RefCell::new(Vec::new());
    p.folder_sizes(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert_eq!(
        sent.borrow().len(),
        1,
        "tree measurement must reach the backend"
    );
    let sent = sent.borrow();
    let Command::Tree(command) = &sent[0] else {
        panic!("tree request")
    };
    let TreeCommand::Sizes(TreeSizeCommand::Compute(request)) = command.as_ref() else {
        panic!("size request")
    };
    assert!(matches!(request.scope, TreeSizeScope::SelectionOrVisible));
    let publication = p.panes[0].tree.publication.as_ref().unwrap();
    assert!(Arc::ptr_eq(&request.view, &publication.view));
    assert!(Arc::ptr_eq(&request.selection, &publication.selection));
}

#[test]
fn tree_space_measures_captured_folder_while_insert_only_toggles() {
    for (key, commands) in [(8, 2), (9, 1)] {
        let (mut p, root) = fixture();
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(publication(
                root, 1, true, false,
            )))),
            &|_| true,
        );
        p.focus_display_row(PaneId::Left, 1);
        let sent = RefCell::new(Vec::new());
        p.move_cursor(PaneId::Left, key, &|command| {
            sent.borrow_mut().push(command);
            true
        });
        assert_eq!(p.panes[0].cursor, Some(EntryId(11)));
        assert_eq!(
            sent.borrow().len(),
            commands,
            "Space measures; Insert only toggles"
        );
        if key == 8 {
            assert!(matches!(&sent.borrow()[0], Command::Tree(command)
                if matches!(command.as_ref(), TreeCommand::Sizes(TreeSizeCommand::Compute(request))
                    if matches!(request.scope, TreeSizeScope::Single(TreeRowId(7))))));
        }
    }
}

#[test]
fn tree_size_snapshots_survive_reprojection_and_stale_snapshots_are_acknowledged() {
    let (mut p, root) = fixture();
    let first = publication(root.clone(), 1, true, false);
    let stamp = first.view.stamp;
    p.apply_with_sender(Event::Tree(Box::new(TreeEvent::Published(first))), &|_| {
        true
    });
    let snapshot = Arc::new(
        TreeSizeSnapshot::new(
            stamp,
            1,
            HashMap::from([(
                TreeRowId(11),
                FolderSize {
                    bytes: 123,
                    finished: true,
                    ..Default::default()
                },
            )]),
            HashSet::new(),
        )
        .unwrap(),
    );
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Sizes(TreeSizeEvent::Snapshot {
            sizes: snapshot,
        }))),
        &|_| true,
    );
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 2, true, false,
        )))),
        &|_| true,
    );
    let child = p.panes[0].display_entry(1).unwrap();
    assert_eq!(p.panes[0].folder_size(child).unwrap().bytes, 123);
    let stale_stamp = TreeViewStamp {
        session: TreeSessionId(4),
        ..stamp
    };
    let stale =
        Arc::new(TreeSizeSnapshot::new(stale_stamp, 99, HashMap::new(), HashSet::new()).unwrap());
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Sizes(TreeSizeEvent::Snapshot {
            sizes: stale,
        }))),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert_eq!(
        p.panes[0]
            .folder_size(p.panes[0].display_entry(1).unwrap())
            .unwrap()
            .bytes,
        123
    );
    assert!(sent.borrow().iter().any(|command| matches!(command, Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Sizes(TreeSizeCommand::Acknowledge { stamp, version: 99 }) if *stamp == stale_stamp))));
}

#[test]
fn deferred_tree_space_keeps_original_occurrence_after_caret_moves() {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, true, false,
        )))),
        &|_| true,
    );
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(7)),
        &|_| true
    ));
    p.focus_display_row(PaneId::Left, 2);
    p.move_cursor(PaneId::Left, 8, &|_| true);
    p.focus_display_row(PaneId::Left, 1);
    let published = p.panes[0].tree.publication.as_ref().unwrap();
    let view = published.view.clone();
    let revision = p.panes[0].tree.revision;
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![3]).unwrap());
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            view,
            revision,
            selection,
        })),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Sizes(TreeSizeCommand::Compute(request))
            if matches!(request.scope, TreeSizeScope::Single(TreeRowId(11))))))
    );
}

#[test]
fn tree_selected_size_status_requires_exact_selection_and_clears_on_pending_toggle() {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, true, true,
        )))),
        &|_| true,
    );
    let published = p.panes[0].tree.publication.as_ref().unwrap();
    let view = published.view.clone();
    let selection = published.selection.clone();
    let summary = SelectedFolderSizes {
        folders: 1,
        complete: 1,
        complete_bytes: Some(123),
        ..Default::default()
    };
    let selected = |selection| TreeSizeEvent::Selected {
        view: view.clone(),
        selection_revision: SelectionRevision(0),
        selection,
        version: 1,
        summary,
    };
    let equal_but_foreign =
        Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Sizes(selected(equal_but_foreign)))),
        &|_| true,
    );
    assert!(p.panes[0].selected_folder_sizes().is_none());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Sizes(selected(selection)))),
        &|_| true,
    );
    assert_eq!(p.panes[0].selected_folder_sizes(), Some(summary));
    assert!(
        p.panel_details(PaneId::Left)
            .iter()
            .any(|detail| detail.label == "Selected folder bytes")
    );
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(7)),
        &|_| true
    ));
    assert!(p.panes[0].selected_folder_sizes().is_none());
}

#[test]
fn tree_folder_size_cancel_uses_captured_request_and_stamp() {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, true, false,
        )))),
        &|_| true,
    );
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.folder_sizes(PaneId::Left, &send);
    p.folder_sizes(PaneId::Left, &send);
    let sent = sent.borrow();
    let Command::Tree(command) = &sent[0] else {
        panic!("tree request")
    };
    let TreeCommand::Sizes(TreeSizeCommand::Compute(request)) = command.as_ref() else {
        panic!("size request")
    };
    assert!(matches!(&sent[1], Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Sizes(TreeSizeCommand::Cancel { stamp, request: id })
            if *stamp == request.view.stamp && *id == request.request)));
}

#[test]
fn tree_space_on_file_toggles_and_advances_without_measurement() {
    let (mut p, root) = fixture();
    let initial = publication(root.clone(), 1, true, false);
    let mut rows = initial.view.rows.to_vec();
    let mut file = root.entries[1].clone();
    file.id = EntryId(77);
    rows.insert(
        0,
        TreeDisplayRow {
            id: TreeRowId(77),
            entry: file,
            parent: None,
            depth: 0,
            disclosure: TreeDisclosure::None,
            expanded: false,
            selectable: true,
            navigable: false,
            context_only: false,
            error: None,
            error_details: None,
            original_root_entry: Some(EntryId(11)),
            prefix_end: 0,
            search_match: false,
        },
    );
    let view = Arc::new(
        TreeProjection::new(root, initial.view.stamp, initial.view.layout_revision, rows).unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    let initial = TreePublication::new(view, SelectionRevision(0), selection).unwrap();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(initial))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 1);
    let sent = RefCell::new(Vec::new());
    p.move_cursor(PaneId::Left, 8, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert_eq!(p.panes[0].cursor, Some(EntryId(7)));
    assert_eq!(sent.borrow().len(), 1);
    assert!(matches!(&sent.borrow()[0], Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Select(_))));
    assert!(p.panes[0].error.is_empty());
}

#[test]
fn deferred_full_tree_scan_cannot_measure_a_newer_selection() {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, true, false,
        )))),
        &|_| true,
    );
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(7)),
        &|_| true
    ));
    p.folder_sizes(PaneId::Left, &|_| panic!("wait for captured selection"));
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(11)),
        &|_| true
    ));
    let view = p.panes[0].tree.publication.as_ref().unwrap().view.clone();
    let revision = p.panes[0].tree.revision;
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![3]).unwrap());
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            view,
            revision,
            selection,
        })),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(
        !sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Sizes(TreeSizeCommand::Compute(_)))))
    );
    assert!(p.desktop.status.contains("selection changed"));
}

#[test]
fn select_all_preview_excludes_nested_rows_before_backend_acknowledges() {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, true, false,
        )))),
        &|_| true,
    );
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(11)),
        &|_| true
    ));
    assert!(p.panes[0].selection_visible(p.panes[0].display_entry(1).unwrap()));
    assert!(p.bulk_selection(PaneId::Left, SelectionOperation::All, &|_| true));
    assert!(p.panes[0].selection_visible(p.panes[0].display_entry(0).unwrap()));
    assert!(!p.panes[0].selection_visible(p.panes[0].display_entry(1).unwrap()));
}

#[test]
fn adoption_preserves_root_caret_and_collapse_focuses_the_visible_ancestor() {
    let (mut p, root) = fixture();
    p.focus_display_row(PaneId::Left, 1);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root.clone(),
            1,
            false,
            false,
        )))),
        &|_| true,
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(7)));
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root.clone(),
            2,
            true,
            false,
        )))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 2);
    assert_eq!(p.panes[0].cursor, Some(EntryId(11)));
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 3, false, false,
        )))),
        &|_| true,
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(7)));
    assert_eq!(p.panes[0].display_len(), 1);
}

#[test]
fn tree_toggle_and_range_predict_marks_without_native_selection_commands() {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root.clone(),
            1,
            false,
            false,
        )))),
        &|_| true,
    );
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 2, true, false,
        )))),
        &|_| true,
    );
    let sent = RefCell::new(Vec::new());
    let send = |c| {
        sent.borrow_mut().push(c);
        true
    };
    p.select_row_modified(PaneId::Left, 1, true, false, &send);
    assert!(p.panes[0].selection_visible(p.panes[0].display_entry(0).unwrap()));
    assert!(!p.panes[0].selection_visible(p.panes[0].display_entry(1).unwrap()));
    p.select_row_modified(PaneId::Left, 2, false, true, &send);
    assert!(p.panes[0].selection_visible(p.panes[0].display_entry(1).unwrap()));
    assert_eq!(sent.borrow().len(), 2);
    for command in sent.borrow().iter() {
        assert!(
            matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Select(_)))
        );
    }
}

#[test]
fn tree_rectangle_uses_projection_and_cancels_on_structural_publication() {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root.clone(),
            1,
            true,
            false,
        )))),
        &|_| true,
    );
    let capture = p.capture_rectangle(PaneId::Left).unwrap();
    let ranges = SelectionRanges::new(2, vec![SelectionSpan { first: 1, last: 1 }]).unwrap();
    assert!(p.preview_rectangle(PaneId::Left, &capture, ranges, SelectionMode::Replace));
    assert!(p.panes[0].selection_visible(p.panes[0].display_entry(1).unwrap()));
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 2, false, false,
        )))),
        &|_| true,
    );
    assert!(!p.rectangle_current(PaneId::Left, &capture));
}

#[test]
fn tree_rows_cannot_become_flat_mutation_or_clipboard_authority() {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, true, false,
        )))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 2);
    let send = |_: Command| panic!("tree occurrence escaped into a flat operation");
    p.preferences_loaded(Ok(UserPreferences::default()));
    p.edit_file(PaneId::Left, &send); // Focused descendant is a directory, never an editor target.
    p.start_operation(
        PaneId::Left,
        MutationKind::Delete(DeleteMode::Permanent),
        &|command| {
            assert!(matches!(command, Command::Tree(command)
                if matches!(command.as_ref(), TreeCommand::Delete(request)
                    if request.fallback == Some(TreeRowId(11)))));
            true
        },
    );
    p.copy_files(PaneId::Left, &|command| {
        assert!(matches!(command, Command::Tree(command)
            if matches!(command.as_ref(), TreeCommand::Clipboard(request)
                if request.fallback == Some(TreeRowId(11)))));
        true
    });
    p.request_copy_dialog(PaneId::Left, &|command| {
        assert!(matches!(command, Command::Tree(command)
            if matches!(command.as_ref(), TreeCommand::Transfer(request)
                if request.fallback == Some(TreeRowId(11)))));
        true
    });
    p.request_checksums(PaneId::Left, &|command| {
        assert!(matches!(command, Command::Tree(command)
            if matches!(command.as_ref(), TreeCommand::Checksums(request)
                if request.fallback == Some(TreeRowId(11)))));
        true
    });
    assert!(
        p.checksums.sessions.is_empty(),
        "tree selection must wait for its backend capture, never an empty flat fallback"
    );
    assert!(p.desktop.notice.is_some());
    assert!(
        p.panes[0].native_capabilities(),
        "location capabilities are independent of action capture"
    );
}

#[test]
fn root_file_activation_uses_original_alias_even_when_descendant_id_matches_native_root() {
    let (mut p, root) = fixture();
    let source = publication(root.clone(), 1, true, false);
    let mut rows = source.view.rows.to_vec();
    let mut file = root.entries[1].clone();
    let path = file.path.clone();
    file.id = EntryId(77);
    rows.push(TreeDisplayRow {
        id: TreeRowId(77),
        entry: file,
        parent: None,
        depth: 0,
        disclosure: TreeDisclosure::None,
        expanded: false,
        selectable: true,
        navigable: false,
        context_only: false,
        error: None,
        error_details: None,
        original_root_entry: Some(EntryId(11)),
        prefix_end: 0,
        search_match: false,
    });
    let view = Arc::new(
        TreeProjection::new(root, source.view.stamp, source.view.layout_revision, rows).unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    let publication = TreePublication::new(view, SelectionRevision(0), selection).unwrap();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    let sent = RefCell::new(Vec::new());
    assert!(p.open(PaneId::Left, 3, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    assert!(
        matches!(&sent.borrow()[0], Command::OpenEntry { path: captured, .. } if captured == &path)
    );
    sent.borrow_mut().clear();
    p.focus_display_row(PaneId::Left, 3);
    assert!(!p.open_folder(PaneId::Left, &|_| panic!(
        "folder-only navigation launched a root file"
    )));
    assert!(p.open(PaneId::Left, 2, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    assert!(
        matches!(&sent.borrow()[0], Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Navigate { row: TreeRowId(11), .. }))
    );
}

#[test]
fn pending_toggle_survives_loading_publication_and_is_reissued_after_commit() {
    let (mut p, root) = fixture();
    let first = publication(root.clone(), 1, false, false);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(first.clone()))),
        &|_| true,
    );
    p.select_row_modified(PaneId::Left, 1, true, false, &|_| true);
    let source = publication(root, 2, false, false);
    let view = Arc::new(
        TreeProjection::new(
            source.view.root.clone(),
            source.view.stamp,
            first.view.layout_revision,
            source.view.rows.to_vec(),
        )
        .unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    let published = TreePublication::new(view.clone(), SelectionRevision(0), selection).unwrap();
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(published))),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(
        p.panes[0].selection_visible(p.panes[0].display_entry(0).unwrap()),
        "loading must not erase a pending selection mark"
    );
    assert!(sent.borrow().iter().any(|command| matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Select(request) if Arc::ptr_eq(&request.view, &view)))));
}

#[test]
fn newer_flat_selection_fences_adoption_and_reseeds_the_exact_root() {
    let (mut p, root) = fixture();
    let sent = RefCell::new(Vec::new());
    p.select_row_modified(PaneId::Left, 1, true, false, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let Command::Selection(request) = sent.borrow_mut().pop().unwrap() else {
        panic!("flat selection");
    };
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation: root.navigation,
        revision: request.revision,
        selection: Arc::new(SelectionSnapshot::from_bits(root.index.clone(), vec![1]).unwrap()),
        deferred: Default::default(),
    });
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root.clone(),
            1,
            false,
            false,
        )))),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(
        p.panes[0].tree_view().is_none(),
        "old empty seed cannot erase a newer selected folder"
    );
    assert!(sent.borrow().iter().any(|command| matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Enable { selection_revision: SelectionRevision(1), selected: TreeRootSelection::Native(selection), .. } if Arc::ptr_eq(selection.basis(), &root.index) && selection.len() == 1))));
    let mut next = publication(root, 2, false, true);
    next.selection_revision = SelectionRevision(1);
    p.apply_with_sender(Event::Tree(Box::new(TreeEvent::Published(next))), &|_| true);
    assert!(p.panes[0].selection_visible(p.panes[0].display_entry(0).unwrap()));
}

#[test]
fn archive_tree_adoption_preserves_virtual_identity_and_uses_typed_navigation() {
    let mut p = super::archives::archive_pane();
    let entries = p.panes[0].entries.clone();
    let identity = p.panes[0].row_identity(&entries[0]);
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.disclose_tree(PaneId::Left, &identity, &send);
    let Command::Tree(command) = sent.borrow_mut().pop().unwrap() else {
        panic!("tree enable");
    };
    let TreeCommand::Enable { root, selected, .. } = *command else {
        panic!("tree enable");
    };
    assert!(matches!(selected, TreeRootSelection::Archive(_)));
    let mut rows = Vec::new();
    for (position, entry) in entries.iter().enumerate() {
        let mut display = entry.clone();
        display.id = EntryId(100 + position as u64);
        rows.push(TreeDisplayRow {
            id: TreeRowId(display.id.0),
            entry: display,
            parent: None,
            depth: 0,
            disclosure: if position == 0 {
                TreeDisclosure::Unknown
            } else {
                TreeDisclosure::None
            },
            expanded: false,
            selectable: true,
            navigable: position == 0,
            context_only: false,
            error: None,
            error_details: None,
            original_root_entry: Some(entry.id),
            prefix_end: 0,
            search_match: false,
        });
    }
    let stamp = TreeViewStamp {
        pane: PaneId::Left,
        navigation: root.navigation,
        session: TreeSessionId(9),
        revision: TreeRevision(1),
    };
    let view = Arc::new(TreeProjection::new(root, stamp, TreeLayoutRevision(1), rows).unwrap());
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(
            TreePublication::new(view, SelectionRevision(0), selection).unwrap(),
        ))),
        &send,
    );
    assert!(p.panes[0].display_entry(0).unwrap().native_path().is_none());
    assert_eq!(
        p.panes[0]
            .display_entry(0)
            .unwrap()
            .virtual_node
            .as_ref()
            .unwrap()
            .id
            .mount,
        42
    );
    sent.borrow_mut().clear();
    assert!(p.open(PaneId::Left, 1, &send));
    assert!(
        matches!(&sent.borrow()[0], Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Navigate { row: TreeRowId(100), .. }))
    );
}

#[test]
fn rejected_selection_releases_rectangle_input_without_reusing_the_failed_revision() {
    let (mut p, root) = fixture();
    let publication = publication(root, 1, false, false);
    let stamp = publication.view.stamp;
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    p.select_row_modified(PaneId::Left, 1, true, false, &|_| true);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionFailed {
            stamp,
            revision: SelectionRevision(1),
            error: TreeDiagnostic::new("busy"),
        })),
        &|_| true,
    );
    assert!(p.capture_rectangle(PaneId::Left).is_some());
    let sent = RefCell::new(None);
    p.select_row_modified(PaneId::Left, 1, true, false, &|command| {
        *sent.borrow_mut() = Some(command);
        true
    });
    assert!(
        matches!(sent.borrow().as_ref(), Some(Command::Tree(command)) if matches!(command.as_ref(), TreeCommand::Select(request) if request.revision == SelectionRevision(2)))
    );
}

#[test]
fn controller_commit_rejection_retries_exact_commit_after_outer_queue_acceptance() {
    let (mut p, root) = fixture();
    let publication = publication(root, 1, false, false);
    let view = publication.view.clone();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Rejected {
            stamp: view.stamp,
            request: TreeRequestId(0),
            row: TreeRowId(0),
            error: TreeDiagnostic::new("queue busy"),
        })),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Commit(actual) if Arc::ptr_eq(actual, &view))))
    );
}

#[test]
fn leaving_tree_uses_fresh_flat_selection_context_even_when_navigation_fails() {
    let (mut p, root) = fixture();
    let old_context = p.panes[0].selection_committed_context;
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, false, false,
        )))),
        &|_| true,
    );
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(p.navigate(PaneId::Left, test_root().join("outside-tree"), None, &send));
    assert_ne!(
        p.panes[0].selection_context, old_context,
        "navigation must not reuse the backend's pre-tree marks"
    );
    let navigation = NavigationId(p.panes[0].generation);
    let path = p.panes[0].path.clone();
    p.apply_with_sender(
        Event::ListingFailed {
            pane: PaneId::Left,
            navigation,
            error: FsError::from_io("refresh", &path, io::ErrorKind::PermissionDenied.into()),
        },
        &send,
    );
    assert!(sent.borrow().iter().any(|command| matches!(command, Command::Selection(request)
        if request.context != old_context && request.restore.as_ref().is_none_or(|origin| origin.navigation != old_context))));
}

#[test]
fn starting_search_while_enable_is_pending_keeps_flat_search_in_control() {
    let (mut p, root) = fixture();
    p.change_search_query(PaneId::Left, "f", &|_| true);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, false, false,
        )))),
        &|_| true,
    );
    assert!(
        p.panes[0].tree_view().is_none(),
        "late Enable must not take over an active flat search"
    );
    p.change_search_query(PaneId::Left, "fo", &|_| true);
    assert_eq!(p.panes[0].search.query, "fo");
}

#[test]
fn refused_insert_toggle_keeps_the_same_occurrence_focused_for_retry() {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, true, false,
        )))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 1);
    p.move_cursor(PaneId::Left, 9, &|_| false);
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(7)),
        "a refused mark must not skip to another occurrence"
    );
    assert!(!p.panes[0].selection_visible(p.panes[0].display_entry(0).unwrap()));
}

#[test]
fn adopted_tree_root_file_can_launch_viewer_and_capture_editor() {
    let (mut p, root) = fixture();
    p.preferences_loaded(Ok(UserPreferences::default()));
    let mut entry = root.entries[1].clone();
    entry.id = EntryId(42); // Collides with the folder's original ID.
    let stamp = TreeViewStamp {
        pane: root.pane,
        navigation: root.navigation,
        session: TreeSessionId(5),
        revision: TreeRevision(1),
    };
    let rows = vec![TreeDisplayRow {
        id: TreeRowId(42),
        entry,
        parent: None,
        depth: 0,
        disclosure: TreeDisclosure::None,
        expanded: false,
        selectable: true,
        navigable: false,
        context_only: false,
        error: None,
        error_details: None,
        original_root_entry: Some(EntryId(11)),
        prefix_end: 0,
        search_match: false,
    }];
    let view =
        Arc::new(TreeProjection::new(root.clone(), stamp, TreeLayoutRevision(1), rows).unwrap());
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(
            TreePublication::new(view, SelectionRevision(0), selection).unwrap(),
        ))),
        &|_| true,
    );
    p.panes[0].cursor = Some(EntryId(42));
    p.panes[0].parent_cursor = false;
    let launch = p
        .viewer_launch(PaneId::Left, true)
        .expect("root alias viewer");
    assert!(Arc::ptr_eq(&launch.entries, &root.entries));
    assert!(Arc::ptr_eq(&launch.view, &root.index));
    assert_eq!(launch.focused, root.entries[1].path);
    assert!(launch.basic);
    assert!(Arc::ptr_eq(
        &launch.associations,
        &p.viewer_plugins.startup_associations
    ));
    assert!(Arc::ptr_eq(
        &launch.disabled_plugins,
        p.viewer_plugins.startup_disabled.as_ref().unwrap()
    ));
    let captured = launch.tree.as_ref().unwrap();
    assert!(Arc::ptr_eq(&captured.view, p.panes[0].tree_view().unwrap()));
    assert_eq!(captured.focused, TreeRowId(42));
    assert!(!p.viewer_launch(PaneId::Left, false).unwrap().basic);
    p.panes[0].loading = true;
    assert!(p.viewer_launch(PaneId::Left, false).is_none());
    p.panes[0].loading = false;
    p.panes[0].parent_cursor = true;
    assert!(p.viewer_launch(PaneId::Left, false).is_none());
    p.panes[0].parent_cursor = false;
    p.panes[0].tree.commit_pending = true;
    assert!(p.viewer_launch(PaneId::Left, false).is_none());
    p.panes[0].tree.commit_pending = false;
    p.panes[0].search.pending = true;
    assert!(p.viewer_launch(PaneId::Left, false).is_none());
    p.panes[0].search.pending = false;
    let commands = RefCell::new(Vec::new());
    p.edit_file(PaneId::Left, &|command| {
        assert!(!matches!(command, Command::EditFile { .. }));
        commands.borrow_mut().push(command);
        true
    });
    let commands = commands.borrow();
    let [Command::Tree(command)] = commands.as_slice() else {
        panic!("typed editor capture required");
    };
    let TreeCommand::Edit(request) = command.as_ref() else {
        panic!("editor capture required");
    };
    assert_eq!(
        request.row,
        TreeRowId(42),
        "the occurrence is not original EntryId(11)"
    );
    assert!(Arc::ptr_eq(&request.view, p.panes[0].tree_view().unwrap()));
}

#[test]
fn native_tree_rename_requests_focused_capture_without_flat_path_authority() {
    let (mut p, root) = fixture();
    let publication = publication(root, 1, true, true);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 2);
    let sent = RefCell::new(Vec::new());
    p.start_operation(PaneId::Left, MutationKind::Rename, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert_eq!(
        sent.borrow().len(),
        1,
        "F2 must request exact native tree focus, not report unavailable"
    );
    let sent = sent.borrow();
    let [Command::Tree(command)] = sent.as_slice() else {
        panic!("typed capture required")
    };
    let TreeCommand::Rename(request) = command.as_ref() else {
        panic!("Rename capture required")
    };
    assert_eq!(request.row, TreeRowId(11));
    assert!(Arc::ptr_eq(&request.view, p.panes[0].tree_view().unwrap()));
    assert!(
        p.operations.preview.is_none(),
        "native preparation must precede confirmation"
    );
}

#[test]
fn native_tree_create_directory_keeps_pane_root_authority_despite_descendant_focus() {
    let (mut p, root) = fixture();
    let source = p.panes[0].path.clone();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root, 1, true, true,
        )))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 2);
    let sent = RefCell::new(Vec::new());
    p.start_operation(PaneId::Left, MutationKind::CreateDirectory, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let commands = sent.borrow();
    let [Command::PrepareOperation(request)] = commands.as_slice() else {
        panic!("F7 prepares pane location");
    };
    assert_eq!(request.kind, MutationKind::CreateDirectory);
    assert_eq!(request.source.path, source);
    assert_eq!(
        request.focused, None,
        "focused descendant is not the create destination"
    );
}

#[test]
fn retiring_presenter_reclaims_saturated_tree_editor_capture_on_worker() {
    #[derive(Debug)]
    struct Source {
        stamp: TreeViewStamp,
        row: TreeRowId,
        dropped: std::sync::mpsc::Sender<std::thread::ThreadId>,
    }
    impl Drop for Source {
        fn drop(&mut self) {
            let _ = self.dropped.send(std::thread::current().id());
        }
    }
    impl CapturedEditorFile for Source {
        fn stamp(&self) -> TreeViewStamp {
            self.stamp
        }
        fn row(&self) -> TreeRowId {
            self.row
        }
        fn resolve(&self, _: &Cancellation) -> Result<PathBuf, FsError> {
            panic!("UI must not resolve captured native paths")
        }
    }
    let (mut p, root) = fixture();
    p.preferences_loaded(Ok(UserPreferences::default()));
    let mut published = publication(root, 1, true, false);
    let mut rows = published.view.rows.to_vec();
    rows[1].entry.kind = EntryKind::File;
    let view = Arc::new(
        TreeProjection::new(
            published.view.root.clone(),
            published.view.stamp,
            TreeLayoutRevision(1),
            rows,
        )
        .unwrap(),
    );
    published = TreePublication::new(
        view.clone(),
        SelectionRevision(0),
        Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap()),
    )
    .unwrap();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(published))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 2);
    let captured = RefCell::new(None);
    p.edit_file(PaneId::Left, &|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Edit(request) = *command
        {
            *captured.borrow_mut() = Some(request);
        }
        true
    });
    let request = captured.into_inner().expect("typed F4 capture");
    let (sent, received) = std::sync::mpsc::channel();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::EditorCaptured {
            stamp: request.view.stamp,
            request: request.request,
            result: Ok(Arc::new(Source {
                stamp: request.view.stamp,
                row: request.row,
                dropped: sent,
            })),
        })),
        &|_| false,
    );
    p.retire_all();
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    drop(p);
    assert_ne!(
        received
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap(),
        std::thread::current().id()
    );
}

#[path = "tree_rename.rs"]
mod rename;

#[path = "tree_metadata.rs"]
mod metadata;

#[test]
fn tree_navigation_receipt_survives_an_unrelated_new_publication() {
    for rejected in [false, true] {
        let (mut p, root) = fixture();
        let original = publication(root.clone(), 1, true, false);
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(original.clone()))),
            &|_| true,
        );
        let sent = RefCell::new(Vec::new());
        let send = |command| {
            sent.borrow_mut().push(command);
            true
        };
        assert!(p.open_tree_row(PaneId::Left, p.panes[0].parent_offset(), &send));
        let (stamp, request, row, navigation) = sent
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::Tree(command) => match command.as_ref() {
                    TreeCommand::Navigate {
                        view,
                        request,
                        row,
                        navigation,
                    } => Some((view.stamp, *request, *row, *navigation)),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(publication(
                root.clone(),
                2,
                true,
                false,
            )))),
            &send,
        );
        sent.borrow_mut().clear();
        if rejected {
            p.apply_with_sender(
                Event::Tree(Box::new(TreeEvent::Rejected {
                    stamp,
                    request,
                    row,
                    error: TreeDiagnostic::new("navigation refused"),
                })),
                &send,
            );
            assert_eq!(p.panes[0].error, "navigation refused");
            assert!(p.panes[0].tree_input_ready());
        } else {
            let target = root.entries[0].path.clone();
            p.apply_with_sender(
                Event::Tree(Box::new(TreeEvent::Navigated {
                    stamp,
                    request,
                    row,
                    navigation,
                    target: TreeNavigationTarget::Native(target.clone()),
                })),
                &send,
            );
            assert!(sent.borrow().iter().any(
                |command| matches!(command, Command::Navigate { path, .. } if path == &target)
            ));
        }
    }
}
