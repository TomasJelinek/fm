//! UI retries and stale replies retain the original focused occurrence.
use super::*;
#[derive(Debug)]
struct Source {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    row: TreeRowId,
}
impl CapturedRenameTarget for Source {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn row(&self) -> TreeRowId {
        self.row
    }
    fn prepare(
        &self,
        _: &dyn MutationService,
        _: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError> {
        panic!("no UI native preparation")
    }
}
fn setup() -> (Presenter, TreeRootSnapshot) {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root.clone(),
            1,
            true,
            true,
        )))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 2);
    (p, root)
}
fn capture(p: &mut Presenter) -> TreeRenameRequest {
    let sent = RefCell::new(None);
    p.start_operation(PaneId::Left, MutationKind::Rename, &|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Rename(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    sent.into_inner().expect("focused capture")
}
fn reply(request: &TreeRenameRequest) -> Event {
    Event::Tree(Box::new(TreeEvent::RenameCaptured {
        stamp: request.view.stamp,
        request: request.request,
        result: Ok(Arc::new(Source {
            stamp: request.view.stamp,
            request: request.request,
            row: request.row,
        })),
    }))
}
#[test]
fn rename_capture_backpressure_retries_same_focus_and_admits_only_once() {
    let (mut p, _) = setup();
    p.start_operation(PaneId::Left, MutationKind::Rename, &|_| false);
    p.focus_display_row(PaneId::Left, 1);
    let sent = RefCell::new(Vec::new());
    p.flush_tree_rename(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    let Command::Tree(command) = sent.borrow_mut().pop().unwrap() else {
        panic!("capture")
    };
    let TreeCommand::Rename(request) = *command else {
        panic!("Rename")
    };
    assert_eq!(request.row, TreeRowId(11));
    p.apply_with_sender(reply(&request), &|_| false);
    assert!(p.operations.preview.is_none());
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.flush_tree_rename(&send);
    p.flush_tree_rename(&send);
    let commands = sent.borrow();
    let [Command::PrepareCapturedRename(prepared)] = commands.as_slice() else {
        panic!("one admission")
    };
    assert_eq!(prepared.source.row(), request.row);
    assert_eq!(prepared.source.stamp(), request.view.stamp);
    assert_eq!(prepared.source_directory, p.panes[0].path);
}
#[test]
fn rename_capture_reprojection_and_shutdown_cancel_pending_and_ignore_late_reply() {
    for closing in [false, true] {
        let (mut p, root) = setup();
        let request = capture(&mut p);
        if closing {
            p.retire_all();
        } else {
            p.apply_with_sender(
                Event::Tree(Box::new(TreeEvent::Published(publication(
                    root, 2, true, false,
                )))),
                &|_| true,
            );
            p.flush_tree_rename(&|_| panic!("stale capture must not prepare"));
        }
        assert!(request.cancel.is_cancelled());
        p.apply_with_sender(reply(&request), &|_| panic!("late reply must not prepare"));
        assert!(p.operations.preview.is_none());
        assert!(p.operations.pending.is_none());
    }
}
#[test]
fn rename_parent_focus_does_not_create_typed_or_flat_authority() {
    let (mut p, _) = setup();
    p.panes[0].parent_cursor = true;
    p.start_operation(PaneId::Left, MutationKind::Rename, &|_| {
        panic!("parent is navigation")
    });
    assert!(p.operations.tree_rename.is_none());
}

#[test]
fn rename_completion_reconciles_expanded_parent_without_flat_navigation() {
    let (mut p, _) = setup();
    let view = p.panes[0].tree_view().unwrap().clone();
    let request = capture(&mut p);
    p.apply_with_sender(reply(&request), &|_| true);
    let mut progress = (*p.operations.progress[0]).clone();
    progress.state = OperationState::Completed;
    progress.completed = 1;
    progress.renamed_to = Some(p.panes[0].path.join("folder/renamed"));
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
fn menu_rename_uses_frozen_clicked_root_without_changing_child_marks_or_caret() {
    let (mut p, root) = fixture();
    let mut publication = publication(root, 1, true, false);
    publication.selection =
        Arc::new(TreeSelectionSnapshot::from_bits(publication.view.clone(), vec![2]).unwrap());
    let view = publication.view.clone();
    let selection = publication.selection.clone();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 2);
    let caret = p.panes[0].cursor;
    let sent = RefCell::new(Vec::new());
    p.request_tree_rename_target(PaneId::Left, view.clone(), TreeRowId(7), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(sent.borrow().iter().any(|command| matches!(command, Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Rename(request) if request.row == TreeRowId(7) && Arc::ptr_eq(&request.view, &view)))));
    assert_eq!(p.panes[0].cursor, caret);
    assert!(Arc::ptr_eq(
        &p.panes[0].tree.publication.as_ref().unwrap().selection,
        &selection
    ));
}
