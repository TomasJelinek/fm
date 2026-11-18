//! Dispatch refusals tell the presenter whether it may send a capture again.
use super::*;
use midnight_core::*;

fn view(root: &TreeRootSnapshot, revision: u64) -> Arc<TreeProjection> {
    let stamp = TreeViewStamp {
        pane: root.pane,
        navigation: root.navigation,
        session: TreeSessionId(1),
        revision: TreeRevision(revision),
    };
    Arc::new(TreeProjection::new(root.clone(), stamp, TreeLayoutRevision(1), vec![]).unwrap())
}

fn transfer(view: &Arc<TreeProjection>) -> TreeCommand {
    TreeCommand::Transfer(TreeTransferRequest {
        intent: TransferIntent::Copy,
        view: view.clone(),
        request: TreeRequestId(1),
        selection_revision: SelectionRevision(0),
        selection: Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![]).unwrap()),
        fallback: None,
        cancel: Cancellation::default(),
    })
}

/// B8-U3: the worker publishes a newer view to the mailbox before the presenter
/// shows it. The mailbox refuses a capture of the replaced view, and this reply
/// can reach the presenter first, so only its flag says the capture is retried.
#[tokio::test]
async fn a_transfer_for_a_replaced_view_is_refused_as_retryable() {
    let (sender, _events) = tokio::sync::mpsc::channel(4);
    let controller = Controller::new(
        Services {
            shared_native: None,
            qualified_locale: None,
            rename: crate::tree_rename::Retirement::testing(),
            filesystem: Arc::new(super::lifetime_tests::Idle),
            locale: None,
            reports: crate::ReportArena::testing(),
        },
        std::array::from_fn(|_| Arc::new(Semaphore::new(1))),
        Arc::new(Semaphore::new(1)),
        crate::test_snapshots(Default::default()),
        crate::Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        None,
    );
    controller.invalidate(PaneId::Left, NavigationId(1), SortSpec::default());
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        entries: Arc::from([]),
        index: Arc::new(DirectoryIndex::default()),
    };
    let (replaced, current) = (view(&root, 1), view(&root, 2));
    assert!(controller.lanes[PaneId::Left.index()].publish(&current));
    let refusal = controller.submit(transfer(&replaced)).await;
    assert!(
        matches!(&refusal, Some(TreeEvent::Transfer {
            stamp,
            retryable: true,
            result: Err(_),
            ..
        }) if *stamp == replaced.stamp),
        "{refusal:?}"
    );
    controller.shutdown().await;
}
