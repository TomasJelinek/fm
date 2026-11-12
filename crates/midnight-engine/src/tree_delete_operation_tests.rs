//! Captured deletion shares confirmation, job bounds and worker retirement with flat mutations.
use super::*;

#[path = "tree_delete_boundary_tests.rs"]
mod boundaries;
#[path = "tree_delete_native_tests.rs"]
mod native;

#[derive(Debug)]
struct Sources {
    paths: Vec<PathBuf>,
    mode: DeleteMode,
    dropped: Option<std::sync::mpsc::Sender<std::thread::ThreadId>>,
}
impl Drop for Sources {
    fn drop(&mut self) {
        if let Some(sender) = &self.dropped {
            let _ = sender.send(std::thread::current().id());
        }
    }
}
impl CapturedDeleteSources for Sources {
    fn stamp(&self) -> TreeViewStamp {
        TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(7),
            session: TreeSessionId(3),
            revision: TreeRevision(1),
        }
    }
    fn request(&self) -> TreeRequestId {
        TreeRequestId(4)
    }
    fn mode(&self) -> DeleteMode {
        self.mode
    }
    fn root_count(&self) -> usize {
        self.paths.len()
    }
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        self.paths
            .iter()
            .map(|path| service.prepare(path, cancel))
            .collect()
    }
}
fn request(h: &Harness, mode: DeleteMode) -> PrepareCapturedDelete {
    PrepareCapturedDelete {
        id: h.request.id,
        source_directory: h.request.source.path.clone(),
        sources: Arc::new(Sources {
            paths: vec![
                h.request.source.path.join("parent/child/first.txt"),
                h.request.source.path.join("other/second.txt"),
            ],
            mode,
            dropped: None,
        }),
    }
}

#[tokio::test]
async fn captured_tree_delete_prepares_descendants_but_requires_confirmation_for_both_modes() {
    for mode in [DeleteMode::Trash, DeleteMode::Permanent] {
        let mut h = Harness::new(MutationKind::Delete(mode));
        let request = request(&h, mode);
        h.controller.prepare_captured_delete(request).await;
        let preview = h.preview().await;
        assert_eq!(preview.kind, MutationKind::Delete(mode));
        assert_eq!((preview.files, preview.folders), (2, 0));
        assert_eq!(h.calls.prepared.load(Ordering::Relaxed), 2);
        assert!(h.calls.deleted.lock().unwrap().is_empty());
        // Later navigation cannot retarget the already pinned confirmation.
        *h.controller.snapshots.lock().unwrap() = Default::default();
        h.controller.confirm(preview.id, None).await;
        let done = h.finished().await;
        assert_eq!(done.state, OperationState::Completed, "{done:?}");
        assert_eq!((done.total, done.completed), (2, 2));
        assert_eq!(
            *h.calls.deleted.lock().unwrap(),
            vec![
                (h.request.source.path.join("parent/child/first.txt"), mode),
                (h.request.source.path.join("other/second.txt"), mode),
            ]
        );
    }
}

#[tokio::test]
async fn cancelled_captured_confirmation_never_deletes_and_releases_owner_on_worker() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
    let (sent, received) = std::sync::mpsc::channel();
    let request = PrepareCapturedDelete {
        id: h.request.id,
        source_directory: h.request.source.path.clone(),
        sources: Arc::new(Sources {
            paths: vec![h.request.source.path.join("parent/child")],
            mode: DeleteMode::Permanent,
            dropped: Some(sent),
        }),
    };
    h.controller.prepare_captured_delete(request).await;
    h.preview().await;
    h.controller.cancel(h.request.id);
    assert_eq!(h.finished().await.state, OperationState::Cancelled);
    let thread = received.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_ne!(thread, std::thread::current().id());
    h.controller.confirm(h.request.id, None).await;
    assert!(h.calls.deleted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn captured_delete_replay_and_shutdown_retire_sources_without_preparation() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    let (sent, received) = std::sync::mpsc::channel();
    for id in [1, 2] {
        if id == 2 {
            h.controller.shutdown();
        }
        let request = PrepareCapturedDelete {
            id: OperationId(id),
            source_directory: h.request.source.path.clone(),
            sources: Arc::new(Sources {
                paths: vec![h.request.source.path.join("must-not-prepare")],
                mode: DeleteMode::Trash,
                dropped: Some(sent.clone()),
            }),
        };
        h.controller.prepare_captured_delete(request).await;
        assert_ne!(
            received.recv_timeout(Duration::from_secs(2)).unwrap(),
            std::thread::current().id()
        );
    }
    assert_eq!(h.calls.prepared.load(Ordering::Relaxed), 1);
    assert!(h.calls.deleted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn flat_delete_still_rejects_descendant_paths_in_a_forged_native_listing() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
    let entries: Arc<[Entry]> = Arc::from([Entry::new(
        EntryId(7),
        h.request.source.path.join("parent/child.txt"),
        EntryKind::File,
    )]);
    h.request.source.entries = entries.clone();
    h.request.view = Arc::new(DirectoryIndex::from_entries(&entries));
    h.controller.snapshots.lock().unwrap()[0]
        .visible
        .as_mut()
        .unwrap()
        .entries = entries;
    h.controller.prepare(h.request.clone()).await;
    let done = h.finished().await;
    assert_eq!(done.state, OperationState::Failed);
    assert_eq!(h.calls.prepared.load(Ordering::Relaxed), 0);
    assert!(h.calls.deleted.lock().unwrap().is_empty());
}
