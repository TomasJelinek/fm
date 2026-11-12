//! Expanded-tree metadata must enter the exact typed capture path before native preview.
use super::*;

#[test]
fn tree_metadata_open_uses_typed_selection_capture_before_preview() {
    for case in [false, true] {
        let (mut p, root) = fixture();
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(publication(
                root, 1, true, false,
            )))),
            &|_| true,
        );
        p.focus_display_row(PaneId::Left, 2);
        let sent = RefCell::new(Vec::new());
        p.open_metadata(PaneId::Left, case, &|command| {
            sent.borrow_mut().push(command);
            true
        });
        assert_eq!(
            sent.borrow().len(),
            1,
            "metadata must capture the real descendant"
        );
        assert!(matches!(&sent.borrow()[0], Command::Tree(_)));
        assert!(
            p.operations.pending.is_none(),
            "capture cannot execute metadata"
        );
    }
}
#[derive(Debug)]
struct Sources {
    stamp: TreeViewStamp,
    request: TreeRequestId,
}
impl CapturedMetadataSources for Sources {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn root_count(&self) -> usize {
        1
    }
    fn covered_roots(&self) -> &[bool] {
        &[false]
    }
    fn prepare(
        &self,
        _: &dyn MutationService,
        _: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        panic!("UI must never prepare native metadata")
    }
}
fn setup(selected: bool) -> (Presenter, TreeRootSnapshot) {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root.clone(),
            1,
            true,
            selected,
        )))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 2);
    (p, root)
}
fn request(p: &mut Presenter, case: bool) -> TreeMetadataRequest {
    let sent = RefCell::new(None);
    p.open_metadata(PaneId::Left, case, &|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Metadata(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    sent.into_inner().expect("metadata capture")
}
fn reply(request: &TreeMetadataRequest) -> Event {
    Event::Tree(Box::new(TreeEvent::MetadataCaptured {
        stamp: request.view.stamp,
        request: request.request,
        retryable: false,
        result: Ok(Arc::new(Sources {
            stamp: request.view.stamp,
            request: request.request,
        })),
    }))
}
#[test]
fn tree_metadata_capture_retains_exact_marks_and_focus_through_outer_queue_refusal() {
    let (mut p, _) = setup(true);
    p.open_metadata(PaneId::Left, true, &|_| false);
    p.focus_display_row(PaneId::Left, 1);
    let sent = RefCell::new(Vec::new());
    p.flush_tree_metadata(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    let Command::Tree(command) = sent.borrow_mut().pop().unwrap() else {
        panic!("capture")
    };
    let TreeCommand::Metadata(request) = *command else {
        panic!("metadata")
    };
    assert_eq!(
        request.fallback,
        Some(TreeRowId(11)),
        "focus is frozen at intent"
    );
    let publication = p.panes[0].tree.publication.as_ref().unwrap();
    assert!(Arc::ptr_eq(&request.view, &publication.view));
    assert!(Arc::ptr_eq(&request.selection, &publication.selection));
    assert!(request.selection.contains_row(TreeRowId(7)));
    p.apply_with_sender(reply(&request), &|_| false);
    assert!(p.metadata.id.is_none());
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.flush_tree_metadata(&send);
    p.flush_tree_metadata(&send);
    let commands = sent.borrow();
    let [Command::PrepareCapturedMetadata(prepared)] = commands.as_slice() else {
        panic!("one prepare")
    };
    assert_eq!(prepared.sources.stamp(), request.view.stamp);
    assert_eq!(prepared.sources.request(), request.request);
    assert_eq!(p.metadata.id, Some(prepared.id));
    assert!(p.metadata.start_case);
    assert!(
        !p.metadata.can_apply(),
        "opening metadata never authorizes mutation"
    );
}
#[test]
fn tree_metadata_transient_capture_refusal_retries_later_with_new_request_same_authority() {
    let (mut p, _) = setup(false);
    let req = request(&mut p, false);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::MetadataCaptured {
            stamp: req.view.stamp,
            request: req.request,
            retryable: true,
            result: Err(TreeDiagnostic::new("Tree queue is full")),
        })),
        &|_| panic!("no immediate retry from the receipt pump"),
    );
    let sent = RefCell::new(Vec::new());
    p.flush_tree_metadata(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    let Command::Tree(command) = sent.borrow_mut().pop().unwrap() else {
        panic!("retry")
    };
    let TreeCommand::Metadata(retry) = *command else {
        panic!("metadata")
    };
    assert!(retry.request.0 > req.request.0);
    assert!(Arc::ptr_eq(&retry.view, &req.view));
    assert!(Arc::ptr_eq(&retry.selection, &req.selection));
    assert_eq!(retry.fallback, req.fallback);
}
#[test]
fn tree_metadata_reprojection_navigation_and_shutdown_retire_late_captures() {
    for mode in 0..3 {
        let (mut p, root) = setup(false);
        let req = request(&mut p, false);
        match mode {
            0 => {
                p.apply_with_sender(
                    Event::Tree(Box::new(TreeEvent::Published(publication(
                        root, 2, true, false,
                    )))),
                    &|_| true,
                );
            }
            1 => {
                p.navigate(PaneId::Left, test_root().join("different"), None, &|_| true);
            }
            _ => p.retire_all(),
        }
        p.flush_tree_metadata(&|_| panic!("stale capture"));
        assert!(req.cancel.is_cancelled());
        p.apply_with_sender(reply(&req), &|_| panic!("late receipt"));
        assert!(p.metadata.id.is_none());
        assert!(p.operations.pending.is_none());
    }
}
#[test]
fn tree_metadata_parent_focus_cannot_authorize_unmarked_target() {
    let (mut p, _) = setup(false);
    p.panes[0].parent_cursor = true;
    p.open_metadata(PaneId::Left, false, &|_| {
        panic!("parent navigation is not metadata")
    });
    assert!(p.operations.tree_metadata.is_none());
}
#[test]
fn tree_metadata_completion_reconciles_loaded_branches_without_flat_navigation() {
    let (mut p, _) = setup(false);
    let view = p.panes[0].tree_view().unwrap().clone();
    let req = request(&mut p, false);
    p.apply_with_sender(reply(&req), &|_| true);
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
