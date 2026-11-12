use super::*;

fn loaded() -> Presenter {
    let (mut p, root) = super::tree::fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(super::tree::publication(
            root, 1, true, false,
        )))),
        &|_| true,
    );
    p
}

fn flush(p: &mut Presenter, accepted: bool) -> Vec<TreeRefreshRequest> {
    let requests = RefCell::new(Vec::new());
    p.flush_selection(&|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::RefreshLoaded(request) = *command
        {
            requests.borrow_mut().push(request);
            return accepted;
        }
        true
    });
    requests.into_inner()
}

fn complete(p: &mut Presenter, request: &TreeRefreshRequest) {
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Refreshed {
            stamp: request.view.stamp,
            request: request.request,
            result: Ok(()),
        })),
        &|_| true,
    );
}

fn publish_next(p: &mut Presenter, revision: u64) {
    let root = p.panes[0].tree_view().unwrap().root.clone();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(super::tree::publication(
            root, revision, true, false,
        )))),
        &|_| true,
    );
}

fn trigger_refresh(p: &mut Presenter, manual: bool, send: &dyn Fn(Command) -> bool) -> bool {
    if manual {
        p.refresh_location(PaneId::Left, send)
    } else {
        let navigation = p.panes[0].successful_generation.unwrap();
        p.refresh_if_changed(PaneId::Left, navigation, send)
    }
}

#[test]
fn native_watch_and_manual_refresh_preserve_adopted_tree_marks_and_caret() {
    for manual in [false, true] {
        let (mut p, root) = super::tree::fixture();
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(super::tree::publication(
                root, 1, true, true,
            )))),
            &|_| true,
        );
        p.focus_display_row(PaneId::Left, 2);
        let view = p.panes[0].tree_view().unwrap().clone();
        let caret = p.panes[0].cursor;
        let marks = p.panes[0]
            .tree
            .publication
            .as_ref()
            .unwrap()
            .selection
            .clone();
        assert!(marks.contains_row(TreeRowId(7)));
        let sent = RefCell::new(Vec::new());
        assert!(trigger_refresh(&mut p, manual, &|command| {
            sent.borrow_mut().push(command);
            true
        }));
        assert!(
            p.panes[0]
                .tree_view()
                .is_some_and(|current| Arc::ptr_eq(current, &view)),
            "refresh must preserve the adopted tree: manual={manual}"
        );
        assert!(!p.panes[0].loading);
        assert_eq!(p.panes[0].cursor, caret);
        assert!(Arc::ptr_eq(
            &p.panes[0].tree.publication.as_ref().unwrap().selection,
            &marks,
        ));
        let commands = sent.into_inner();
        assert_eq!(commands.len(), 1);
        assert!(matches!(&commands[0], Command::Tree(command)
            if matches!(command.as_ref(), TreeCommand::RefreshLoaded(request)
                if Arc::ptr_eq(&request.view, &view))));
    }
}

#[test]
fn native_watch_and_manual_refresh_retain_dirtiness_when_outer_sender_refuses() {
    for manual in [false, true] {
        let mut p = loaded();
        let view = p.panes[0].tree_view().unwrap().clone();
        assert!(trigger_refresh(&mut p, manual, &|_| false));
        assert!(Arc::ptr_eq(p.panes[0].tree_view().unwrap(), &view));
        let requests = flush(&mut p, true);
        assert_eq!(
            requests.len(),
            1,
            "queued refresh must retry: manual={manual}"
        );
        assert!(Arc::ptr_eq(&requests[0].view, &view));
        assert!(p.operations.refresh.is_empty());
    }
}

#[test]
fn stale_root_watch_hint_does_not_refresh_an_adopted_tree() {
    let mut p = loaded();
    let navigation = p.panes[0].successful_generation.unwrap();
    assert!(
        !p.refresh_if_changed(PaneId::Left, NavigationId(navigation.0 + 1), &|_| {
            panic!("stale hints must not send")
        })
    );
    assert!(flush(&mut p, true).is_empty());
    assert!(p.panes[0].tree_view().is_some());
}

#[test]
fn tree_watch_hints_follow_session_identity_across_projection_revisions() {
    let mut p = loaded();
    let stamp = p.panes[0].tree_view().unwrap().stamp;
    publish_next(&mut p, 2);
    for stale in [
        TreeViewStamp {
            navigation: NavigationId(stamp.navigation.0 + 1),
            ..stamp
        },
        TreeViewStamp {
            session: TreeSessionId(stamp.session.0 + 1),
            ..stamp
        },
    ] {
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Changed { stamp: stale })),
            &|_| panic!("stale hints must not send"),
        );
        assert!(flush(&mut p, true).is_empty());
    }
    p.apply_with_sender(Event::Tree(Box::new(TreeEvent::Changed { stamp })), &|_| {
        false
    });
    let requests = flush(&mut p, true);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].view.stamp.revision, TreeRevision(2));
    assert_eq!(requests[0].view.stamp.session, stamp.session);
}

#[test]
fn watch_scope_failure_is_visible_without_rejecting_commit_or_looping_refresh() {
    let mut p = loaded();
    let stamp = p.panes[0].tree_view().unwrap().stamp;
    publish_next(&mut p, 2);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::WatchFailed {
            stamp,
            error: TreeDiagnostic::new("Native watch budget exhausted"),
        })),
        &|_| panic!("a watch diagnostic must not reject the accepted Commit"),
    );
    assert!(p.panes[0].error.contains("Native watch budget exhausted"));
    assert!(!p.panes[0].tree.commit_pending);
    assert!(flush(&mut p, true).is_empty());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::WatchFailed {
            stamp: TreeViewStamp {
                session: TreeSessionId(stamp.session.0 + 1),
                ..stamp
            },
            error: TreeDiagnostic::new("Retired scope"),
        })),
        &|_| true,
    );
    assert!(!p.panes[0].error.contains("Retired scope"));
}

#[test]
fn tree_refresh_coalesces_and_retries_outer_refusal_without_flat_navigation() {
    let mut p = loaded();
    assert!(p.panes[0].queue_native_tree_refresh(&mut p.retired));
    assert!(p.panes[0].queue_native_tree_refresh(&mut p.retired));
    let refused = flush(&mut p, false).pop().unwrap();
    let request = flush(&mut p, true).pop().unwrap();
    assert_eq!(refused.request, request.request);
    assert!(Arc::ptr_eq(&refused.view, &request.view));
    assert!(flush(&mut p, true).is_empty());
    assert!(p.operations.refresh.is_empty());
    complete(&mut p, &request);
    assert!(flush(&mut p, true).is_empty());
}

#[test]
fn intermediate_publications_do_not_complete_refresh_and_a_later_dirty_epoch_survives() {
    let mut p = loaded();
    p.panes[0].queue_native_tree_refresh(&mut p.retired);
    let first = flush(&mut p, true).pop().unwrap();
    publish_next(&mut p, 2);
    p.panes[0].queue_native_tree_refresh(&mut p.retired);
    assert!(flush(&mut p, true).is_empty());
    complete(&mut p, &first);
    let second = flush(&mut p, true).pop().unwrap();
    assert_eq!(second.view.stamp.revision, TreeRevision(2));
    assert!(second.request.0 > first.request.0);
    complete(&mut p, &first);
    assert!(
        flush(&mut p, true).is_empty(),
        "old completion cannot clear a new request"
    );
    complete(&mut p, &second);
    assert!(flush(&mut p, true).is_empty());
}

#[test]
fn retryable_rejection_rebinds_to_the_current_acknowledged_view() {
    let mut p = loaded();
    p.panes[0].queue_native_tree_refresh(&mut p.retired);
    let first = flush(&mut p, true).pop().unwrap();
    publish_next(&mut p, 2);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::RefreshRejected {
            stamp: first.view.stamp,
            request: first.request,
            error: TreeDiagnostic::new("Busy"),
            retryable: true,
        })),
        &|_| true,
    );
    let retry = flush(&mut p, true).pop().unwrap();
    assert!(Arc::ptr_eq(&retry.view, p.panes[0].tree_view().unwrap()));
    assert_eq!(retry.view.stamp.revision, TreeRevision(2));
    assert!(retry.request.0 > first.request.0);
    complete(&mut p, &retry);
    assert!(flush(&mut p, true).is_empty());
}

#[test]
fn provider_failure_is_visible_and_does_not_loop_until_another_transfer() {
    let mut p = loaded();
    p.panes[0].queue_native_tree_refresh(&mut p.retired);
    let request = flush(&mut p, true).pop().unwrap();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Refreshed {
            stamp: request.view.stamp,
            request: request.request,
            result: Err(TreeDiagnostic::new("Access denied")),
        })),
        &|_| true,
    );
    assert!(p.panes[0].error.contains("Access denied"));
    assert!(flush(&mut p, true).is_empty());
    assert!(p.panes[0].tree_view().is_some());
    p.panes[0].queue_native_tree_refresh(&mut p.retired);
    assert_eq!(flush(&mut p, true).len(), 1);
}

#[test]
fn navigation_retires_dirty_and_inflight_refresh_and_ignores_old_results() {
    let mut p = loaded();
    p.panes[0].queue_native_tree_refresh(&mut p.retired);
    let request = flush(&mut p, true).pop().unwrap();
    p.clear_tree(PaneId::Left, Some(NavigationId(99)));
    complete(&mut p, &request);
    assert!(flush(&mut p, true).is_empty());
    assert!(p.panes[0].error.is_empty());
    assert!(!p.panes[0].queue_native_tree_refresh(&mut p.retired));
}

#[test]
fn refresh_waits_for_pending_user_selection() {
    let mut p = loaded();
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(7)),
        &|_| true,
    ));
    p.panes[0].queue_native_tree_refresh(&mut p.retired);
    assert!(flush(&mut p, true).is_empty());
    let view = p.panes[0].tree_view().unwrap().clone();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            revision: p.panes[0].tree.revision,
            selection: Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap()),
            view,
        })),
        &|_| true,
    );
    assert_eq!(flush(&mut p, true).len(), 1);
}

#[test]
fn refresh_waits_for_successful_root_deselection_acknowledgement() {
    let mut p = loaded();
    let view = p.panes[0].tree_view().unwrap().clone();
    let operation = OperationId(9);
    p.track_copy_selection(operation);
    p.complete_copy_selection(
        operation,
        CompletedCopySource::Tree(TreeCopyOccurrence {
            stamp: view.stamp,
            row: TreeRowId(7),
        }),
    );
    p.panes[0].queue_native_tree_refresh(&mut p.retired);
    assert!(flush(&mut p, true).is_empty());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            revision: p.panes[0].tree.revision,
            selection: Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap()),
            view,
        })),
        &|_| true,
    );
    assert_eq!(flush(&mut p, true).len(), 1);
}

#[test]
fn all_terminal_transfer_outcomes_refresh_native_trees_without_path_sampling() {
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        for state in [
            CopyOperationState::Completed,
            CopyOperationState::Failed,
            CopyOperationState::Cancelled,
            CopyOperationState::CompletedWithErrors,
        ] {
            let mut p = loaded();
            let id = OperationId(1);
            let mut progress = CopyProgress::queued(id, test_root().join("elsewhere"), 1, intent);
            p.copies.progress.push(Arc::new(progress.clone()));
            progress.state = state;
            // No root path is present in the bounded source-directory sample.
            progress.source_directories.clear();
            p.apply(Event::Copy(CopyEvent::Progress(Arc::new(progress))));
            assert_eq!(flush(&mut p, true).len(), 1, "{intent:?} {state:?}");
            assert!(
                p.operations
                    .refresh
                    .iter()
                    .all(|(pane, _, _)| *pane != PaneId::Left)
            );
        }
    }
}

#[test]
fn archive_tree_never_gets_native_maintenance() {
    let mut p = loaded();
    p.panes[0].archive = super::archives::archive_pane().panes[0].archive.take();
    assert!(!p.panes[0].queue_native_tree_refresh(&mut p.retired));
    assert!(flush(&mut p, true).is_empty());
}

#[test]
fn refreshed_root_file_uses_current_viewer_authority_and_cannot_restore_a_foreign_flat_caret() {
    let mut p = loaded();
    let old = p.panes[0].tree_view().unwrap().clone();
    let added = Entry::new(
        EntryId(90),
        p.panes[0].display_path().join("new-žluťoučký.txt"),
        EntryKind::File,
    );
    let mut entries = old.root.entries.to_vec();
    entries.push(added.clone());
    let actions = TreeRootSnapshot {
        entries: entries.clone().into(),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        ..old.root.clone()
    };
    let mut rows = old.rows.to_vec();
    let mut display_entry = added.clone();
    display_entry.id = EntryId(77);
    rows.push(TreeDisplayRow {
        id: TreeRowId(77),
        entry: display_entry,
        parent: None,
        depth: 0,
        disclosure: TreeDisclosure::None,
        expanded: false,
        selectable: true,
        navigable: false,
        context_only: false,
        error: None,
        error_details: None,
        original_root_entry: Some(added.id),
        prefix_end: 0,
        search_match: false,
    });
    let view = Arc::new(
        TreeProjection::new_with_root_actions(
            old.root.clone(),
            actions,
            TreeViewStamp {
                revision: TreeRevision(2),
                ..old.stamp
            },
            TreeLayoutRevision(2),
            rows,
        )
        .unwrap(),
    );
    let publication = TreePublication::new(
        view.clone(),
        SelectionRevision(0),
        Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap()),
    )
    .unwrap();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    p.panes[0].cursor = Some(EntryId(77));
    p.panes[0].parent_cursor = false;
    p.viewer_plugins.startup_disabled = Some(Arc::from([]));
    let launch = p
        .viewer_launch(PaneId::Left, false)
        .expect("new native root file is viewable");
    assert!(Arc::ptr_eq(&launch.entries, &view.root_actions.entries));
    assert!(Arc::ptr_eq(&launch.view, &view.root_actions.index));
    assert_eq!(launch.focused, added.path);
    assert!(!Arc::ptr_eq(&launch.entries, &old.root.entries));
    let commands = RefCell::new(Vec::new());
    let position = p.panes[0].parent_offset() + view.index.row_positions[&TreeRowId(77)];
    assert!(p.open_tree_row(PaneId::Left, position, &|command| {
        commands.borrow_mut().push(command);
        true
    }));
    assert!(commands.borrow().iter().any(|command| {
        matches!(command, Command::OpenEntry { path, .. } if path == &added.path)
    }));
    p.clear_tree(PaneId::Left, Some(NavigationId(99)));
    assert!(
        p.panes[0].cursor.is_none(),
        "new alias is not an old flat entry ID"
    );
}
