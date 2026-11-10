use super::*;
use midnight_core::TreeSessionId;

fn handle() -> Handle {
    Handle {
        panes: std::array::from_fn(|_| {
            Arc::new(Shared {
                state: Mutex::new(Control {
                    visible: None,
                    revision: 0,
                    cancel: Cancellation::default(),
                    stopped: false,
                    navigation: NavigationId(0),
                    tree_navigation: None,
                    tree: None,
                }),
                worker: OnceLock::new(),
                timing: Timing {
                    quiet: Duration::from_secs(1),
                    maximum: Duration::from_secs(3),
                    poll: Duration::from_secs(15),
                },
            })
        }),
    }
}
fn scope(revision: u64) -> TreeScope {
    TreeScope {
        stamp: TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            session: TreeSessionId(1),
            revision: TreeRevision(revision),
        },
        targets: Vec::new(),
        eligible: HashSet::new(),
        complete: true,
    }
}
fn epochs(handle: &Handle) -> (u64, u64) {
    let state = handle.panes[0].state.lock().unwrap();
    let tree = state.tree.as_ref().unwrap();
    (tree.epoch, tree.acknowledged)
}
fn dirty(handle: &Handle) {
    handle.panes[0]
        .state
        .lock()
        .unwrap()
        .tree
        .as_mut()
        .unwrap()
        .dirty();
}

#[test]
fn success_fence_clears_only_admitted_epoch_on_sufficient_exact_commit() {
    let handle = handle();
    handle.set_tree_scope(scope(1));
    dirty(&handle);
    dirty(&handle);
    let ticket = handle
        .begin_refresh(scope(1).stamp, TreeRequestId(1))
        .unwrap();
    dirty(&handle);
    handle.finish_refresh(ticket, true, TreeRevision(2));
    handle.set_tree_scope(scope(1));
    assert_eq!(epochs(&handle), (3, 0));
    handle.set_tree_scope(scope(2));
    assert_eq!(epochs(&handle), (3, 2));
    handle.set_tree_scope(scope(3));
    assert_eq!(epochs(&handle), (3, 2));
}

#[test]
fn failed_scope_commit_never_acknowledges_success_fence() {
    let handle = handle();
    handle.set_tree_scope(scope(1));
    dirty(&handle);
    let ticket = handle
        .begin_refresh(scope(1).stamp, TreeRequestId(1))
        .unwrap();
    handle.finish_refresh(ticket, true, TreeRevision(2));
    let error = TreeDiagnostic::new("admission failed");
    assert!(handle.scope_failed(scope(2).stamp, HashSet::new(), error.clone()));
    assert_eq!(epochs(&handle), (2, 0));
    assert!(!handle.scope_failed(scope(2).stamp, HashSet::new(), error));
    assert_eq!(epochs(&handle), (2, 0));
}

#[test]
fn stale_scope_ticket_cannot_affect_next_navigation() {
    let handle = handle();
    handle.set_tree_scope(scope(1));
    dirty(&handle);
    let ticket = handle
        .begin_refresh(scope(1).stamp, TreeRequestId(1))
        .unwrap();
    handle.clear_scope(PaneId::Left, NavigationId(2));
    assert!(
        handle
            .begin_refresh(scope(1).stamp, TreeRequestId(2))
            .is_none()
    );
    let mut next = scope(1);
    next.stamp.navigation = NavigationId(2);
    next.stamp.session = TreeSessionId(2);
    handle.set_tree_scope(next.clone());
    dirty(&handle);
    handle.finish_refresh(ticket, true, TreeRevision(1));
    handle.set_tree_scope(next);
    assert_eq!(epochs(&handle), (1, 0));
}

#[test]
fn dirty_epoch_exhaustion_is_reported_without_wrapping() {
    let handle = handle();
    handle.set_tree_scope(scope(1));
    {
        let mut state = handle.panes[0].state.lock().unwrap();
        state.tree.as_mut().unwrap().epoch = u64::MAX;
    }
    dirty(&handle);
    let state = handle.panes[0].state.lock().unwrap();
    let tree = state.tree.as_ref().unwrap();
    assert_eq!(tree.epoch, u64::MAX);
    assert!(tree.exhausted);
}

#[test]
fn late_old_session_retirement_does_not_cancel_new_flat_navigation() {
    let handle = handle();
    let old = scope(1).stamp;
    handle.set_tree_scope(scope(1));
    handle.clear_scope(PaneId::Left, NavigationId(2));
    let controller = Controller {
        panes: handle.panes.clone(),
    };
    controller.set_visible(VisibleDirectory {
        pane: PaneId::Left,
        navigation: NavigationId(2),
        path: PathBuf::from("next"),
        entries: Arc::default(),
    });
    assert!(!handle.panes[0].state.lock().unwrap().cancel.is_cancelled());
    assert!(
        handle.begin_refresh(old, TreeRequestId(1)).is_none(),
        "an old retained scope cannot admit a ticket against the new live cancellation token"
    );
    // A blocked old probe can still own the old scope while the new flat
    // generation has already been installed through PaneCommitted.
    handle.retire_session(old);
    assert!(
        !handle.panes[0].state.lock().unwrap().cancel.is_cancelled(),
        "late fatal failure from the old model must not cancel the new navigation"
    );
}
