//! Captured native Move uses exact actor occurrences and ordinary per-file removal.
use super::*;
use crate::{
    providers::{NativeProvider, ProviderRegistry},
    tree_actor::{Session, WorkerBudget},
    tree_projection::ProjectionOptions,
};

fn capture_paths(
    paths: &[PathBuf],
    intent: TransferIntent,
    capacity: &Arc<Semaphore>,
) -> (TreeTransferCapture, std::sync::Weak<ProviderRegistry>) {
    let cancel = Cancellation::default();
    let entries: Arc<[Entry]> = paths
        .iter()
        .enumerate()
        .map(|(i, path)| {
            Entry::new(
                EntryId(i as u64 + 1),
                path.clone(),
                if path.is_dir() {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
            )
        })
        .collect::<Vec<_>>()
        .into();
    let native = NativeProvider::new(Arc::new(midnight_platform::LocalFileSystem::new(None)));
    let opened = native
        .capture_root(paths[0].parent().unwrap(), &entries, &cancel)
        .unwrap();
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(17),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let registry = Arc::new(ProviderRegistry::default());
    let weak = Arc::downgrade(&registry);
    let mut session = Session::new(
        root,
        opened,
        registry,
        WorkerBudget::new(Arc::new(Semaphore::new(2))),
        ProjectionOptions::default(),
        &cancel,
    )
    .unwrap();
    let publication = session.publication().unwrap();
    assert!(session.commit(&publication.view));
    session
        .select(&TreeSelectionRequest {
            view: publication.view,
            revision: SelectionRevision(1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action: TreeSelectionAction::Bulk(SelectionOperation::All),
        })
        .unwrap();
    let publication = session.publication().unwrap();
    assert!(session.commit(&publication.view));
    let captured = session
        .capture_transfer(
            &TreeTransferRequest {
                intent,
                view: publication.view,
                request: TreeRequestId(1),
                selection_revision: publication.selection_revision,
                selection: publication.selection,
                fallback: None,
                cancel: cancel.clone(),
            },
            capacity,
            &cancel,
        )
        .unwrap();
    (captured, weak)
}

#[tokio::test]
async fn tree_move_root_feedback_waits_for_native_rename_and_skip_retains_later_root() {
    let (_temp, parent) = moves::directories::tree_fixture();
    let first = parent.join("a-tree");
    let second = parent.join("b-file");
    let destination = parent.join("target");
    fs::create_dir_all(first.join("nested/empty")).unwrap();
    fs::write(first.join("nested/file"), b"tree bytes").unwrap();
    fs::write(&second, b"retain bytes").unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("b-file"), b"winner").unwrap();
    let capacity = Arc::new(Semaphore::new(4));
    let (capture, registration) = capture_paths(
        &[first.clone(), second.clone()],
        TransferIntent::Move,
        &capacity,
    );
    let mut h = Harness::native();
    h.controller
        .enqueue_captured(CapturedTransferRequest {
            verification: midnight_core::TransferVerification::default(),
            id: OperationId(1),
            destination: destination.clone(),
            sources: capture.sources.clone(),
            metadata: TransferMetadataPolicy::ContentOnly,
        })
        .await;
    drop(capture);
    let mut completed = Vec::new();
    loop {
        match h.next().await {
            CopyEvent::RootCompleted {
                source: CompletedCopySource::Tree(occurrence),
                ..
            } => {
                assert!(
                    !first.exists(),
                    "root feedback must follow directory removal and stage cleanup"
                );
                assert_eq!(
                    fs::read(destination.join("a-tree/nested/file")).unwrap(),
                    b"tree bytes"
                );
                assert!(destination.join("a-tree/nested/empty").is_dir());
                completed.push(occurrence);
            }
            CopyEvent::Conflict(conflict) => {
                assert_eq!(completed.len(), 1);
                h.controller
                    .decide(CopyDecision {
                        id: conflict.id,
                        version: conflict.version,
                        choice: CopyChoice::Skip,
                    })
                    .await;
            }
            CopyEvent::Progress(p) if p.state.is_terminal() => {
                assert_eq!(p.state, CopyOperationState::CompletedWithErrors, "{p:?}");
                assert_eq!(p.source_files_removed, 0);
                assert_eq!(p.native_entries_moved, 1);
                assert_eq!(p.copied_bytes + p.verified_bytes + p.validation_bytes, 0);
                break;
            }
            CopyEvent::Failure(f) => panic!("unexpected failure: {f:?}"),
            _ => {}
        }
    }
    assert_eq!(completed.len(), 1);
    assert_eq!(fs::read(second).unwrap(), b"retain bytes");
    assert_eq!(fs::read(destination.join("b-file")).unwrap(), b"winner");
    h.close().await;
    assert_eq!(capacity.available_permits(), 4);
    assert!(registration.upgrade().is_none());
}

#[test]
fn tree_move_descendant_capture_keeps_provider_and_permit_after_source_close() {
    let (_temp, parent) = moves::directories::tree_fixture();
    let root = parent.join("root");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("payload"), b"child").unwrap();
    let capacity = Arc::new(Semaphore::new(4));
    let (captured, registration) = capture_paths(&[root], TransferIntent::Move, &capacity);
    let provider = midnight_platform::LocalFileSystem::new(None);
    let PreparedCapturedTransfer::Move(mut roots) = captured
        .sources
        .prepare(&provider, &Cancellation::default())
        .unwrap()
    else {
        panic!("Move authority required")
    };
    let mut root = roots.pop_front().unwrap();
    assert!(root.source.tree_copy_occurrence().is_some());
    let child = root
        .source
        .next_move_child(&Cancellation::default())
        .unwrap()
        .unwrap();
    assert!(child.source.tree_copy_occurrence().is_none());
    drop(captured);
    drop(root);
    drop(child.source);
    assert!(registration.upgrade().is_some());
    assert_eq!(capacity.available_permits(), 3);
    child
        .capture
        .preflight_finalization(&Cancellation::default())
        .unwrap();
    drop(child.capture);
    assert!(registration.upgrade().is_none());
    assert_eq!(capacity.available_permits(), 4);
}

#[test]
fn tree_copy_and_move_share_capture_capacity_and_move_root_budget() {
    let (_temp, parent) = moves::protected_fixture();
    let source = parent.join("file");
    fs::write(&source, b"source").unwrap();
    let capacity = Arc::new(Semaphore::new(4));
    let mut captures = Vec::new();
    for intent in [
        TransferIntent::Copy,
        TransferIntent::Move,
        TransferIntent::Copy,
        TransferIntent::Move,
    ] {
        captures.push(capture_paths(std::slice::from_ref(&source), intent, &capacity).0);
    }
    assert_eq!(capacity.available_permits(), 0);
    drop(captures);
    assert_eq!(capacity.available_permits(), 4);
    let paths: Vec<_> = (0..65)
        .map(|n| {
            let path = parent.join(format!("file-{n}"));
            fs::write(&path, b"file").unwrap();
            path
        })
        .collect();
    let (captured, _) = capture_paths(&paths, TransferIntent::Move, &capacity);
    let provider = midnight_platform::LocalFileSystem::new(None);
    assert!(
        captured
            .sources
            .prepare(&provider, &Cancellation::default())
            .is_err()
    );
    assert!(paths.iter().all(|path| path.exists()));
}
