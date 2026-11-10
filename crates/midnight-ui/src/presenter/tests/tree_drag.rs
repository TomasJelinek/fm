use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn loaded() -> Presenter {
    let (mut p, root) = super::tree::fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(super::tree::publication(
            root, 1, true, true,
        )))),
        &|_| true,
    );
    p
}

fn start(p: &mut Presenter) -> (DragId, TreeClipboardRequest) {
    let identity = p.panes[0].row_identity(&p.panes[0].tree_row(1).unwrap().entry);
    let sent = RefCell::new(Vec::new());
    assert!(p.begin_file_drag(PaneId::Left, 2, &identity, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    let id = DragId(p.drag_drop.lifecycle.active.unwrap().id);
    let request = sent
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Tree(command) => match *command {
                TreeCommand::Clipboard(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("exact tree capture");
    (id, request)
}

#[derive(Debug)]
struct Files(Arc<AtomicUsize>);

impl CapturedClipboardFiles for Files {
    fn intent(&self) -> FileClipboardIntent {
        FileClipboardIntent::Copy
    }
    fn resolve(&self, _: &Cancellation) -> Result<FileClipboard, FsError> {
        panic!("UI must not resolve a drag source")
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn receipt(request: &TreeClipboardRequest, drops: Arc<AtomicUsize>) -> Event {
    Event::Tree(Box::new(TreeEvent::Clipboard {
        stamp: request.view.stamp,
        request: request.request,
        result: Ok(Arc::new(Files(drops))),
    }))
}

fn reclaim(p: &mut Presenter, drops: &AtomicUsize) {
    assert_eq!(drops.load(Ordering::SeqCst), 0, "last owner dropped on UI");
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn tree_drag_preserves_exact_marks_and_captures_the_unselected_dragged_row() {
    let mut p = loaded();
    let selection = p.panes[0]
        .tree
        .publication
        .as_ref()
        .unwrap()
        .selection
        .clone();
    let (_, request) = start(&mut p);
    assert_eq!(request.scope, TreeClipboardScope::Dragged(TreeRowId(11)));
    assert!(Arc::ptr_eq(&request.selection, &selection));
    assert!(request.selection.contains_position(0));
    assert!(!request.selection.contains_position(1));
    assert!(Arc::ptr_eq(
        &p.panes[0].tree.publication.as_ref().unwrap().selection,
        &selection
    ));
}

#[test]
fn tree_drag_backpressure_retains_source_then_admits_once_without_cancelling_it() {
    let mut p = loaded();
    let (id, request) = start(&mut p);
    let drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(receipt(&request, drops.clone()), &|_| false);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let sent = RefCell::new(Vec::new());
    p.flush_drag_commands(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert_eq!(
        sent.borrow()
            .iter()
            .filter(|command| matches!(command,
        Command::PrepareCapturedDrag { id: actual, pane: PaneId::Left, navigation, .. }
        if *actual == id && *navigation == request.view.stamp.navigation))
            .count(),
        1
    );
    assert!(!request.cancel.is_cancelled());
    assert!(p.drag_drop.tree.source.is_none());
    sent.borrow_mut().clear();
    p.flush_drag_commands(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(
        !sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::PrepareCapturedDrag { .. }))
    );
    reclaim(&mut p, &drops);
}

#[test]
fn ending_gesture_or_navigation_cancels_pending_capture_and_retires_late_source() {
    for navigate in [false, true] {
        let mut p = loaded();
        let (_, request) = start(&mut p);
        if navigate {
            p.clear_tree(PaneId::Left, Some(NavigationId(77)));
        } else {
            p.release_file_drag(None, InternalDropIntent::Copy, &|_| true);
        }
        assert!(request.cancel.is_cancelled());
        let drops = Arc::new(AtomicUsize::new(0));
        p.apply_with_sender(receipt(&request, drops.clone()), &|command| {
            assert!(!matches!(command, Command::PrepareCapturedDrag { .. }));
            true
        });
        reclaim(&mut p, &drops);
    }
}

#[test]
fn tree_drag_waits_for_the_acknowledged_selection_without_reconstructing_a_snapshot() {
    let mut p = loaded();
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(11)),
        &|_| true
    ));
    let revision = p.panes[0].tree.revision;
    let view = p.panes[0].tree_view().unwrap().clone();
    let identity = p.panes[0].row_identity(&view.rows[1].entry);
    assert!(p.begin_file_drag(PaneId::Left, 2, &identity, &|_| panic!(
        "selection not acknowledged"
    )));
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![3]).unwrap());
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
        sent.borrow().iter().any(|command| matches!(command,
        Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Clipboard(request)
        if request.scope == TreeClipboardScope::Dragged(TreeRowId(11))
        && request.selection_revision == revision && Arc::ptr_eq(&request.selection, &selection))))
    );
}

#[test]
fn tree_parent_and_stale_row_identity_cannot_start_dragging() {
    let mut p = loaded();
    let identity = p.panes[0].row_identity(&p.panes[0].tree_row(1).unwrap().entry);
    assert!(!p.begin_file_drag(PaneId::Left, 0, &identity, &|_| panic!("parent captured")));
    assert!(!p.begin_file_drag(PaneId::Left, 2, "stale", &|_| panic!("stale captured")));
}

fn ready(p: &mut Presenter) -> DragId {
    let (id, request) = start(p);
    p.apply_with_sender(receipt(&request, Arc::default()), &|_| true);
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::Prepared { id, item_count: 1 }),
        &|_| true,
    );
    id
}

fn release_to_child(p: &mut Presenter) -> (DragId, TreeViewStamp, TreeRequestId) {
    let id = ready(p);
    let identity = p.panes[0].row_identity(&p.panes[0].tree_row(1).unwrap().entry);
    let before = p.panes[0].tree.request;
    let target = p
        .preview_internal_drag_target(PaneId::Left, Some((2, &identity)))
        .unwrap();
    assert!(matches!(
        &target,
        InternalDropDestination::TreeOccurrence {
            request: TreeRequestId(0),
            ..
        }
    ));
    assert_eq!(
        p.panes[0].tree.request, before,
        "hover must not allocate requests"
    );
    let sent = RefCell::new(Vec::new());
    p.release_file_drag(Some(target), InternalDropIntent::Move, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let sent = sent.borrow();
    let request = sent
        .iter()
        .find_map(|command| match command {
            Command::InternalDrop(request) => Some(request),
            _ => None,
        })
        .expect("typed internal drop");
    assert_eq!(request.id, id);
    assert_eq!(request.intent, InternalDropIntent::Move);
    let InternalDropDestination::TreeOccurrence {
        view,
        row: TreeRowId(11),
        request,
    } = &request.destination
    else {
        panic!("exact destination occurrence expected")
    };
    assert!(request.0 > before.0);
    (id, view.stamp, *request)
}

#[derive(Debug)]
struct Folder {
    stamp: TreeViewStamp,
    row: TreeRowId,
    drops: Arc<AtomicUsize>,
}
impl CapturedDropDirectory for Folder {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn row(&self) -> TreeRowId {
        self.row
    }
    fn resolve(&self, _: &Cancellation) -> Result<PathBuf, FsError> {
        panic!("UI must not resolve a destination")
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn folder_receipt(
    id: DragId,
    stamp: TreeViewStamp,
    request: TreeRequestId,
    row: TreeRowId,
    drops: Arc<AtomicUsize>,
) -> Event {
    Event::Tree(Box::new(TreeEvent::DropDirectoryCaptured {
        stamp,
        request,
        id,
        result: Ok(Arc::new(Folder { stamp, row, drops })),
    }))
}

#[test]
fn tree_drop_waits_for_exact_destination_and_retries_handoff_under_backpressure() {
    let mut p = loaded();
    let (id, stamp, request) = release_to_child(&mut p);
    let drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(
        folder_receipt(id, stamp, request, TreeRowId(11), drops.clone()),
        &|_| false,
    );
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let sent = RefCell::new(Vec::new());
    p.flush_drag_commands(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(sent.borrow().iter().any(|command| matches!(command,
        Command::CompleteInternalDrop { id: actual, target }
        if *actual == id && target.stamp() == stamp && target.row() == TreeRowId(11))));
    assert!(p.drag_drop.tree.destination.is_none());
    sent.borrow_mut().clear();
    reclaim(&mut p, &drops);
}

#[test]
fn tree_drop_rejects_stale_navigation_or_a_mismatched_capture_without_resolving_paths() {
    for navigate in [false, true] {
        let mut p = loaded();
        let (id, stamp, request) = release_to_child(&mut p);
        if navigate {
            p.clear_tree(PaneId::Left, Some(NavigationId(88)));
        }
        let drops = Arc::new(AtomicUsize::new(0));
        p.apply_with_sender(
            folder_receipt(id, stamp, request, TreeRowId(7), drops.clone()),
            &|command| {
                assert!(!matches!(command, Command::CompleteInternalDrop { .. }));
                true
            },
        );
        assert!(p.drag_drop.tree.destination.is_none());
        reclaim(&mut p, &drops);
    }
}
