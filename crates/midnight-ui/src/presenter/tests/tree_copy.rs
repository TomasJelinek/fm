use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn loaded() -> Presenter {
    let (mut p, root) = super::tree::fixture();
    let mut publication = super::tree::publication(root, 1, true, false);
    publication.selection =
        Arc::new(TreeSelectionSnapshot::from_bits(publication.view.clone(), vec![2]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    p
}

fn request(p: &mut Presenter) -> TreeTransferRequest {
    request_intent(p, TransferIntent::Copy)
}

fn request_intent(p: &mut Presenter, intent: TransferIntent) -> TreeTransferRequest {
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    match intent {
        TransferIntent::Copy => p.request_copy_dialog(PaneId::Left, &send),
        TransferIntent::Move => p.request_move_dialog(PaneId::Left, &send),
    }
    commands
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Tree(command) => match *command {
                TreeCommand::Transfer(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("F5/F6 must capture the selected nested occurrence")
}

#[derive(Debug)]
struct Sources(Arc<AtomicUsize>, TransferIntent);
impl Drop for Sources {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl CapturedTransferSources for Sources {
    fn intent(&self) -> TransferIntent {
        self.1
    }
    fn root_count(&self) -> usize {
        1
    }
    fn metadata_available(&self) -> bool {
        false
    }
    fn prepare(
        &self,
        _: &dyn TransferService,
        _: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        panic!("UI must never prepare a transfer source")
    }
}

fn captured(p: &mut Presenter, request: TreeTransferRequest, drops: Arc<AtomicUsize>) {
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Transfer {
            stamp: request.view.stamp,
            request: request.request,
            result: Ok(TreeTransferCapture {
                intent: request.intent,
                root_count: 1,
                metadata_available: false,
                sources: Arc::new(Sources(drops, request.intent)),
            }),
        })),
        &|_| true,
    );
}

fn enqueued() -> (Presenter, TreeViewStamp, OperationId) {
    let mut p = loaded();
    let request = request(&mut p);
    let stamp = request.view.stamp;
    captured(&mut p, request, Arc::default());
    let draft = p.copies.draft.as_ref().unwrap();
    let token = draft.token.clone();
    let destination = draft.destination.to_str().unwrap().to_owned();
    p.confirm_copy_dialog(&token, &destination, &|_| true);
    let id = p.copies.progress[0].id;
    (p, stamp, id)
}

fn complete(p: &mut Presenter, stamp: TreeViewStamp, id: OperationId) {
    p.apply(Event::Copy(CopyEvent::RootCompleted {
        id,
        source: CompletedCopySource::Tree(TreeCopyOccurrence {
            stamp,
            row: TreeRowId(11),
        }),
    }));
}

#[test]
fn tree_copy_captures_nested_selection_and_survives_navigation_in_draft() {
    let mut p = loaded();
    let original = p.panes[0].tree_view().unwrap().clone();
    let destination = p.panes[1].display_path().to_owned();
    let request = request(&mut p);
    assert!(Arc::ptr_eq(&original, &request.view));
    assert!(request.selection.contains_position(1));
    assert!(!request.selection.contains_position(0));
    assert!(p.copies.draft.is_none());
    captured(&mut p, request, Arc::default());
    let token = p.copies.draft.as_ref().unwrap().token.clone();
    assert!(!p.copies.draft.as_ref().unwrap().metadata_available());
    p.clear_tree(PaneId::Left, Some(NavigationId(99)));
    p.panes[1].path = destination.join("another location");
    let sent = RefCell::new(None);
    p.confirm_copy_dialog(&token, destination.to_str().unwrap(), &|command| {
        *sent.borrow_mut() = Some(command);
        true
    });
    assert!(
        matches!(sent.into_inner(), Some(Command::EnqueueCapturedTransfer(request))
        if request.destination == destination && request.sources.root_count() == 1)
    );
    assert!(p.copies.draft.is_none());
}

#[test]
fn tree_copy_cancel_and_stale_result_reclaim_capture_on_worker() {
    for stale in [false, true] {
        let mut p = loaded();
        let request = request(&mut p);
        if stale {
            p.clear_tree(PaneId::Left, Some(NavigationId(99)));
        }
        let drops = Arc::new(AtomicUsize::new(0));
        captured(&mut p, request, drops.clone());
        if stale {
            assert!(p.copies.draft.is_none());
        } else {
            let token = p.copies.draft.as_ref().unwrap().token.clone();
            p.cancel_copy_dialog(&token);
        }
        assert_eq!(drops.load(Ordering::SeqCst), 0, "capture dropped by UI");
        let retired = p.take_retired();
        std::thread::spawn(move || drop(retired)).join().unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn tree_copy_refused_confirmation_keeps_original_draft_and_capture() {
    let mut p = loaded();
    let request = request(&mut p);
    let drops = Arc::new(AtomicUsize::new(0));
    captured(&mut p, request, drops.clone());
    let draft = p.copies.draft.as_ref().unwrap();
    let token = draft.token.clone();
    let destination = draft.destination.to_str().unwrap().to_owned();
    p.confirm_copy_dialog(&token, &destination, &|_| false);
    assert_eq!(p.copies.draft.as_ref().unwrap().token, token);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert!(p.copies.progress.is_empty());
}

#[test]
fn tree_copy_waits_for_requested_selection_revision_before_capture() {
    let mut p = loaded();
    p.panes[0].cursor = Some(EntryId(11));
    p.panes[0].parent_cursor = false;
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(11)),
        &|_| true
    ));
    let revision = p.panes[0].tree.revision;
    let view = p.panes[0].tree_view().unwrap().clone();
    let sent = RefCell::new(Vec::new());
    p.request_copy_dialog(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(
        sent.borrow().is_empty(),
        "F5 must wait for settled selection"
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            view,
            revision,
            selection: selection.clone(),
        })),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(sent.borrow().iter().any(|command| matches!(command,
        Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Transfer(request)
        if request.selection_revision == revision && Arc::ptr_eq(&request.selection, &selection)
            && request.fallback == Some(TreeRowId(11))))));
}

#[test]
fn tree_transfer_parent_without_selection_remains_unavailable() {
    for moving in [false, true] {
        let mut p = loaded();
        let publication = p.panes[0].tree.publication.as_mut().unwrap();
        publication.selection =
            Arc::new(TreeSelectionSnapshot::from_bits(publication.view.clone(), vec![0]).unwrap());
        p.panes[0].parent_cursor = true;
        p.panes[0].cursor = None;
        let sent = RefCell::new(0);
        let send = |_: Command| {
            *sent.borrow_mut() += 1;
            true
        };
        if moving {
            p.request_move_dialog(PaneId::Left, &send);
        } else {
            p.request_copy_dialog(PaneId::Left, &send);
        }
        assert_eq!(*sent.borrow(), 0);
        assert!(p.copies.draft.is_none());
    }
}

#[test]
fn tree_copy_prepared_result_survives_a_new_projection_of_the_same_session() {
    let mut p = loaded();
    let request = request(&mut p);
    let root = request.view.root.clone();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(super::tree::publication(
            root, 2, true, false,
        )))),
        &|_| true,
    );
    captured(&mut p, request, Arc::default());
    assert!(
        p.copies.draft.is_some(),
        "a captured provider lease outlives display-only changes"
    );
}

#[test]
fn tree_copy_closing_before_result_discards_capture_without_opening_dialog() {
    let mut p = loaded();
    let request = request(&mut p);
    let cancel = request.cancel.clone();
    p.request_operation_shutdown(&|_| true);
    assert!(cancel.is_cancelled());
    let drops = Arc::new(AtomicUsize::new(0));
    captured(&mut p, request, drops.clone());
    assert!(p.copies.draft.is_none());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn tree_copy_after_a_rejected_selection_uses_last_acknowledged_selection() {
    let mut p = loaded();
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(11)),
        &|_| true
    ));
    let stamp = p.panes[0].tree_view().unwrap().stamp;
    let revision = p.panes[0].tree.revision;
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionFailed {
            stamp,
            revision,
            error: TreeDiagnostic::new("transient refusal"),
        })),
        &|_| true,
    );
    let expected = p.panes[0]
        .tree
        .publication
        .as_ref()
        .unwrap()
        .selection
        .clone();
    let request = request(&mut p);
    assert_eq!(request.selection_revision, SelectionRevision(0));
    assert!(Arc::ptr_eq(&request.selection, &expected));
}

#[test]
fn tree_copy_completion_removes_only_completed_occurrence_and_retries_refusal() {
    let (mut p, stamp, id) = enqueued();
    let cursor = p.panes[0].cursor;
    p.apply(Event::Copy(CopyEvent::RootCompleted {
        id,
        source: CompletedCopySource::Tree(TreeCopyOccurrence {
            stamp,
            row: TreeRowId(11),
        }),
    }));
    p.flush_selection(&|_| false);
    let sent = RefCell::new(None);
    p.flush_selection(&|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Select(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    let request = sent
        .into_inner()
        .expect("completed nested root must be deselected");
    assert!(
        matches!(&request.action, TreeSelectionAction::Remove(rows) if rows.as_ref() == [TreeRowId(11)])
    );
    assert_eq!(p.panes[0].cursor, cursor);
    assert!(!p.panes[0].selection_visible(&request.view.rows[1].entry));
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionFailed {
            stamp,
            revision: request.revision,
            error: TreeDiagnostic::new("busy"),
        })),
        &|_| false,
    );
    let retried = RefCell::new(false);
    p.flush_selection(&|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Select(retry) = *command
        {
            assert!(retry.revision > request.revision);
            *retried.borrow_mut() = true;
        }
        true
    });
    assert!(*retried.borrow());
}

#[test]
fn tree_copy_completion_waits_for_user_selection_and_preserves_other_marks() {
    let (mut p, stamp, id) = enqueued();
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(7)),
        &|_| true
    ));
    complete(&mut p, stamp, id);
    p.flush_selection(&|_| panic!("completion must wait for the preceding user edit"));
    let view = p.panes[0].tree_view().unwrap().clone();
    let revision = p.panes[0].tree.revision;
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            view: view.clone(),
            revision,
            selection: Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![3]).unwrap()),
        })),
        &|_| true,
    );
    p.flush_selection(&|_| true);
    assert!(p.panes[0].selection_visible(&view.rows[0].entry));
    assert!(!p.panes[0].selection_visible(&view.rows[1].entry));
}

#[test]
fn tree_copy_completion_never_clears_another_session_with_the_same_row_ids() {
    let (mut p, stamp, id) = enqueued();
    let old = p.panes[0].tree_view().unwrap().clone();
    let view = Arc::new(
        TreeProjection::new(
            old.root.clone(),
            TreeViewStamp {
                session: TreeSessionId(stamp.session.0 + 1),
                ..stamp
            },
            old.layout_revision,
            old.rows.to_vec(),
        )
        .unwrap(),
    );
    let publication = TreePublication::new(
        view.clone(),
        SelectionRevision(0),
        Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![2]).unwrap()),
    )
    .unwrap();
    p.panes[0].tree.publication = Some(publication);
    complete(&mut p, stamp, id);
    p.flush_copy_selection(&|_| panic!("old completion reached a new tree session"));
    assert!(p.panes[0].selection_visible(&view.rows[1].entry));
}

#[test]
fn tree_copy_completion_rebinds_inflight_removal_after_reprojection() {
    let (mut p, stamp, id) = enqueued();
    complete(&mut p, stamp, id);
    p.flush_selection(&|_| true);
    let old_view = p.panes[0].tree_view().unwrap().clone();
    let old_revision = p.panes[0].tree.revision;
    let mut publication = super::tree::publication(old_view.root.clone(), 2, true, false);
    let view = publication.view.clone();
    publication.selection =
        Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![3]).unwrap());
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &send,
    );
    // A delayed old ACK must not acknowledge the retry bound to the new projection.
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            view: old_view.clone(),
            revision: old_revision,
            selection: Arc::new(TreeSelectionSnapshot::from_bits(old_view, vec![0]).unwrap()),
        })),
        &send,
    );
    p.flush_selection(&send);
    let requests = sent.into_inner();
    let retry = requests
        .iter()
        .find_map(|command| match command {
            Command::Tree(command) => match command.as_ref() {
                TreeCommand::Select(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("inflight completion must be reapplied to the new projection");
    assert!(Arc::ptr_eq(&retry.view, &view));
    assert!(retry.revision > old_revision);
    assert!(matches!(&retry.action, TreeSelectionAction::Remove(rows)
        if rows.as_ref() == [TreeRowId(11)]));
    assert!(p.panes[0].selection_visible(&view.rows[0].entry));
    assert!(!p.panes[0].selection_visible(&view.rows[1].entry));
}

#[test]
fn native_tree_f6_requests_transfer_capture_before_opening_dialog() {
    let mut p = loaded();
    let sent = RefCell::new(Vec::new());
    p.request_move_dialog(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Transfer(request)
            if request.intent == TransferIntent::Move && request.selection.len() == 1))),
        "F6 must capture the selected native descendant instead of reporting unavailable"
    );
    assert!(
        p.copies.draft.is_none(),
        "dialog must wait for backend capture"
    );
}

#[test]
fn tree_move_draft_retains_intent_and_targets_after_navigation_and_queue_refusal() {
    let mut p = loaded();
    let request = request_intent(&mut p, TransferIntent::Move);
    captured(&mut p, request, Arc::default());
    let draft = p.copies.draft.as_ref().unwrap();
    assert_eq!(draft.intent, TransferIntent::Move);
    let token = draft.token.clone();
    let destination = draft.destination.clone();
    p.clear_tree(PaneId::Left, Some(NavigationId(99)));
    p.panes[1].path = test_root().join("new destination");
    p.confirm_copy_dialog(&token, destination.to_str().unwrap(), &|_| false);
    assert_eq!(
        p.copies.draft.as_ref().unwrap().intent,
        TransferIntent::Move
    );
    assert!(p.copies.progress.is_empty());
    let sent = RefCell::new(None);
    p.confirm_copy_dialog(&token, destination.to_str().unwrap(), &|command| {
        *sent.borrow_mut() = Some(command);
        true
    });
    assert!(
        matches!(sent.into_inner(), Some(Command::EnqueueCapturedTransfer(request))
        if request.sources.intent() == TransferIntent::Move && request.destination == destination)
    );
    assert_eq!(p.copies.progress[0].intent, TransferIntent::Move);
}

#[test]
fn tree_capture_rejects_mismatched_intent_and_retires_sources_on_worker() {
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        for changed_source in [false, true] {
            let mut p = loaded();
            let request = request_intent(&mut p, intent);
            let other = match intent {
                TransferIntent::Copy => TransferIntent::Move,
                TransferIntent::Move => TransferIntent::Copy,
            };
            let drops = Arc::new(AtomicUsize::new(0));
            p.apply_with_sender(
                Event::Tree(Box::new(TreeEvent::Transfer {
                    stamp: request.view.stamp,
                    request: request.request,
                    result: Ok(TreeTransferCapture {
                        intent: if changed_source { intent } else { other },
                        root_count: 1,
                        metadata_available: false,
                        sources: Arc::new(Sources(
                            drops.clone(),
                            if changed_source { other } else { intent },
                        )),
                    }),
                })),
                &|_| true,
            );
            assert!(
                p.copies.draft.is_none(),
                "a capture cannot change the requested operation"
            );
            assert!(p.desktop.notice.is_some());
            assert_eq!(drops.load(Ordering::SeqCst), 0);
            let retired = p.take_retired();
            std::thread::spawn(move || drop(retired)).join().unwrap();
            assert_eq!(drops.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn tree_copy_confirmation_cannot_upgrade_its_capture_to_move() {
    let mut p = loaded();
    let request = request(&mut p);
    captured(&mut p, request, Arc::default());
    let draft = p.copies.draft.as_mut().unwrap();
    draft.intent = TransferIntent::Move;
    let token = draft.token.clone();
    let destination = draft.destination.to_str().unwrap().to_owned();
    p.confirm_copy_dialog(&token, &destination, &|_| {
        panic!("mismatched draft was enqueued")
    });
    assert!(!p.copies.draft.as_ref().unwrap().error.is_empty());
    assert!(p.copies.progress.is_empty());
}

#[test]
fn archive_tree_move_is_refused_without_reinterpreting_its_native_display_path() {
    let mut p = loaded();
    p.panes[0].archive = super::archives::archive_pane().panes[0].archive.take();
    p.request_move_dialog(PaneId::Left, &|_| {
        panic!("archive Move reached native capture")
    });
    assert!(p.copies.draft.is_none());
    assert!(p.copies.tree_pending.is_none());
    assert_eq!(p.desktop.notice.as_ref().unwrap().0, "Move unavailable");
}
