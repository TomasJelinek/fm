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
fn request(p: &mut Presenter, mode: DeleteMode) -> TreeDeleteRequest {
    let sent = RefCell::new(Vec::new());
    p.start_operation(PaneId::Left, MutationKind::Delete(mode), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    sent.into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Tree(command) => match *command {
                TreeCommand::Delete(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("Delete must capture exact tree occurrences")
}
#[derive(Debug)]
struct Source {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    mode: DeleteMode,
    drops: Arc<AtomicUsize>,
}
impl CapturedDeleteSources for Source {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn mode(&self) -> DeleteMode {
        self.mode
    }
    fn root_count(&self) -> usize {
        1
    }
    fn prepare(
        &self,
        _: &dyn MutationService,
        _: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        panic!("UI must never prepare native deletion")
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn receipt(request: &TreeDeleteRequest, drops: Arc<AtomicUsize>) -> Event {
    Event::Tree(Box::new(TreeEvent::DeleteCaptured {
        stamp: request.view.stamp,
        request: request.request,
        result: Ok(TreeDeleteCapture {
            mode: request.mode,
            root_count: 1,
            sources: Arc::new(Source {
                stamp: request.view.stamp,
                request: request.request,
                mode: request.mode,
                drops,
            }),
        }),
    }))
}
fn reclaim(p: &mut Presenter, drops: &AtomicUsize) {
    assert_eq!(drops.load(Ordering::SeqCst), 0, "provider destroyed on UI");
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn tree_delete_prepares_exact_occurrences_and_requires_confirmation_in_both_modes() {
    for mode in [DeleteMode::Trash, DeleteMode::Permanent] {
        let mut p = loaded();
        let source_directory = p.panes[0].path.clone();
        let request = request(&mut p, mode);
        assert!(request.selection.contains_position(1));
        assert!(p.operations.progress.is_empty());
        p.active = PaneId::Right;
        let sent = RefCell::new(Vec::new());
        p.apply_with_sender(receipt(&request, Arc::default()), &|command| {
            sent.borrow_mut().push(command);
            true
        });
        let commands = sent.into_inner();
        let [Command::PrepareCapturedDelete(prepared)] = commands.as_slice() else {
            panic!()
        };
        assert_eq!(prepared.source_directory, source_directory);
        assert_eq!(prepared.sources.stamp(), request.view.stamp);
        assert_eq!(prepared.sources.mode(), mode);
        let id = prepared.id;
        assert!(
            crate::job_presentation::rows(&p).is_empty(),
            "unconfirmed work is not a job"
        );
        p.apply(Event::Operation(OperationEvent::Preview(Arc::new(
            OperationPreview {
                id,
                kind: MutationKind::Delete(mode),
                files: 0,
                folders: 1,
                file_bytes: Some(0),
                sample: vec![],
            },
        ))));
        p.confirm_operation("wrong", String::new(), &|_| panic!("wrong token"));
        p.confirm_operation(&id.0.to_string(), String::new(), &|command| {
            assert!(matches!(command, Command::ConfirmOperation { id: confirmed, new_name: None } if confirmed == id));
            true
        });
        assert_eq!(crate::job_presentation::rows(&p).len(), 1);
        assert!(p.operations.preview.is_none());
    }
}

#[test]
fn tree_delete_backpressure_and_failed_prepare_do_not_create_jobs() {
    let mut p = loaded();
    p.start_operation(
        PaneId::Left,
        MutationKind::Delete(DeleteMode::Trash),
        &|_| false,
    );
    assert!(p.operations.tree_delete.is_some());
    let sent = RefCell::new(Vec::new());
    p.flush_tree_delete(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    let Command::Tree(command) = sent.borrow_mut().remove(0) else {
        panic!()
    };
    let TreeCommand::Delete(request) = *command else {
        panic!()
    };
    let drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(receipt(&request, drops.clone()), &|_| false);
    assert!(p.operations.tree_delete.is_none());
    assert!(p.operations.pending.is_none());
    assert!(p.operations.progress.is_empty());
    assert!(p.desktop.notice.is_some());
    reclaim(&mut p, &drops);
}

#[test]
fn tree_delete_navigation_reprojection_shutdown_and_replacement_reject_late_captures() {
    for cause in 0..4 {
        let mut p = loaded();
        let request = request(&mut p, DeleteMode::Permanent);
        match cause {
            0 => p.clear_tree(PaneId::Left, Some(NavigationId(99))),
            1 => p.apply_with_sender(
                Event::Tree(Box::new(TreeEvent::Published(super::tree::publication(
                    request.view.root.clone(),
                    2,
                    true,
                    false,
                )))),
                &|_| true,
            ),
            2 => p.request_operation_shutdown(&|_| true),
            _ => {
                p.start_operation(PaneId::Left, MutationKind::CreateFile, &|_| true);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        p.apply_with_sender(receipt(&request, drops.clone()), &|command| {
            assert!(!matches!(command, Command::PrepareCapturedDelete(_)));
            true
        });
        assert!(request.cancel.is_cancelled());
        reclaim(&mut p, &drops);
    }
}

#[test]
fn tree_delete_invalid_capture_payload_never_prepares_a_native_operation() {
    for mismatch in 0..4 {
        let mut p = loaded();
        let request = request(&mut p, DeleteMode::Permanent);
        let drops = Arc::new(AtomicUsize::new(0));
        let mut event = receipt(&request, drops.clone());
        if let Event::Tree(event) = &mut event
            && let TreeEvent::DeleteCaptured {
                result: Ok(captured),
                ..
            } = event.as_mut()
        {
            match mismatch {
                0 => captured.mode = DeleteMode::Trash,
                1 => captured.root_count = 0,
                2 => captured.root_count = 2,
                _ => {
                    captured.sources = Arc::new(Source {
                        stamp: request.view.stamp,
                        request: TreeRequestId(request.request.0 + 1),
                        mode: request.mode,
                        drops: Arc::default(),
                    })
                }
            }
        }
        p.apply_with_sender(event, &|command| {
            assert!(!matches!(command, Command::PrepareCapturedDelete(_)));
            true
        });
        assert!(p.operations.progress.is_empty());
        assert!(request.cancel.is_cancelled());
        if mismatch < 3 {
            reclaim(&mut p, &drops);
        }
    }
}

#[test]
fn tree_delete_waits_for_acknowledged_selection_and_never_targets_parent() {
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
    p.start_operation(
        PaneId::Left,
        MutationKind::Delete(DeleteMode::Trash),
        &|_| panic!("before ack"),
    );
    p.panes[0].cursor = Some(EntryId(7));
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    let sent = RefCell::new(Vec::new());
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
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Delete(request)
            if request.selection_revision == revision && Arc::ptr_eq(&request.selection, &selection)
                && request.fallback == Some(TreeRowId(11)))))
    );
    p.cancel_tree_delete(None);
    p.panes[0].cursor = None;
    p.panes[0].parent_cursor = true;
    p.start_operation(
        PaneId::Left,
        MutationKind::Delete(DeleteMode::Trash),
        &|_| panic!("parent"),
    );
    assert!(p.operations.tree_delete.is_none());
}

#[test]
fn tree_delete_completion_refreshes_loaded_branches_without_flat_navigation() {
    let mut p = loaded();
    let view = p.panes[0].tree_view().unwrap().clone();
    let request = request(&mut p, DeleteMode::Permanent);
    p.apply_with_sender(receipt(&request, Arc::default()), &|_| true);
    let mut progress = (*p.operations.progress[0]).clone();
    progress.state = OperationState::Completed;
    progress.completed = 1;
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::Operation(OperationEvent::Progress(Arc::new(progress))),
        &|command| {
            assert!(!matches!(
                command,
                Command::Navigate {
                    pane: PaneId::Left,
                    ..
                } | Command::Refresh {
                    pane: PaneId::Left,
                    ..
                }
            ));
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(Arc::ptr_eq(p.panes[0].tree_view().unwrap(), &view));
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::RefreshLoaded(_))))
    );
}

#[test]
fn tree_delete_failed_previous_cancel_or_id_exhaustion_retires_capture_without_admission() {
    for exhausted in [false, true] {
        let mut p = loaded();
        p.operations.pending = Some(OperationId(40));
        p.operations.next_id = if exhausted { u64::MAX } else { 40 };
        let request = request(&mut p, DeleteMode::Trash);
        let drops = Arc::new(AtomicUsize::new(0));
        p.apply_with_sender(receipt(&request, drops.clone()), &|command| {
            assert!(matches!(command, Command::CancelOperation(OperationId(40))));
            exhausted
        });
        assert!(request.cancel.is_cancelled());
        assert_eq!(p.operations.pending, Some(OperationId(40)));
        assert!(p.operations.progress.is_empty());
        reclaim(&mut p, &drops);
    }
}

#[test]
fn tree_delete_confirmation_cancel_leaves_no_job_and_ignores_duplicate_capture() {
    let mut p = loaded();
    let request = request(&mut p, DeleteMode::Trash);
    p.apply_with_sender(receipt(&request, Arc::default()), &|_| true);
    let id = p.operations.pending.unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(receipt(&request, drops.clone()), &|_| {
        panic!("duplicate capture")
    });
    reclaim(&mut p, &drops);
    p.cancel_operation(&id.0.to_string(), &|command| {
        assert!(matches!(command, Command::CancelOperation(cancelled) if cancelled == id));
        true
    });
    let mut progress = (*p.operations.progress[0]).clone();
    progress.state = OperationState::Cancelled;
    p.apply(Event::Operation(OperationEvent::Progress(Arc::new(
        progress,
    ))));
    assert!(crate::job_presentation::rows(&p).is_empty());
    assert!(p.operations.pending.is_none());
    assert!(p.operations.progress.is_empty());
}

#[test]
fn completed_menu_capture_forks_only_while_view_and_settled_selection_are_unchanged() {
    for changed in 0..6 {
        let mut p = loaded();
        let mut original = tree_capture::Capture::new(&p.panes[0]).unwrap();
        original.request = Some(TreeRequestId(99));
        assert!(original.matches_current(&p.panes[0]));
        let forked = original.fork_if_current(&p.panes[0]).unwrap();
        assert!(forked.request.is_none());
        assert!(Arc::ptr_eq(&forked.view, &original.view));
        assert_eq!(forked.fallback, original.fallback);
        match changed {
            0 => p.panes[0].loading = true,
            1 => p.panes[0].tree.commit_pending = true,
            2 => p.panes[0].tree.applied = SelectionRevision(1),
            3 => p.panes[0].tree.preview.push((
                SelectionRevision(1),
                TreeSelectionAction::Toggle(TreeRowId(7)),
            )),
            4 => {
                let root = original.view.root.clone();
                p.apply_with_sender(
                    Event::Tree(Box::new(TreeEvent::Published(super::tree::publication(
                        root, 2, true, false,
                    )))),
                    &|_| true,
                );
            }
            _ => original.cancel.cancel(),
        }
        assert!(!original.matches_current(&p.panes[0]));
        assert!(original.fork_if_current(&p.panes[0]).is_none());
        if changed == 5 {
            assert!(
                !forked.cancel.is_cancelled(),
                "the menu and new action own independent cancellation"
            );
        }
    }
}

#[test]
fn scoped_menu_delete_keeps_clicked_row_separate_from_marks_and_caret() {
    let mut p = loaded();
    p.focus_display_row(PaneId::Left, 2);
    let caret = p.panes[0].cursor;
    let selected = p.panes[0]
        .tree
        .publication
        .as_ref()
        .unwrap()
        .selection
        .clone();
    let capture = tree_capture::Capture::new(&p.panes[0]).unwrap();
    let sent = RefCell::new(Vec::new());
    p.start_tree_delete_capture(
        DeleteMode::Trash,
        capture,
        TreeClipboardScope::Dragged(TreeRowId(7)),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    let commands = sent.into_inner();
    let request = commands
        .iter()
        .find_map(|command| match command {
            Command::Tree(command) => match command.as_ref() {
                TreeCommand::Delete(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("scoped menu delete");
    assert_eq!(request.scope, TreeClipboardScope::Dragged(TreeRowId(7)));
    assert!(Arc::ptr_eq(&request.selection, &selected));
    assert_eq!(p.panes[0].cursor, caret);
    assert!(selected.contains_row(TreeRowId(11)));
    assert!(!selected.contains_row(TreeRowId(7)));
}

#[test]
fn invalid_tree_rename_does_not_cancel_an_existing_delete_capture() {
    let mut p = loaded();
    let existing = request(&mut p, DeleteMode::Trash);
    p.panes[0].parent_cursor = true;
    p.panes[0].cursor = None;
    p.start_operation(PaneId::Left, MutationKind::Rename, &|_| {
        panic!("the parent row has no rename target")
    });
    assert!(!existing.cancel.is_cancelled());
    assert!(p.operations.tree_delete.is_some());
}
