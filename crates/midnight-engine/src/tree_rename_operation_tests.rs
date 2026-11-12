//! Captured Rename reuses native preparation, confirmation and versioned conflict decisions.
use super::*;
#[path = "tree_rename_native_tests.rs"]
mod native;

#[derive(Debug)]
struct Source {
    path: PathBuf,
    row: TreeRowId,
}
impl CapturedRenameTarget for Source {
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
    fn row(&self) -> TreeRowId {
        self.row
    }
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError> {
        service.prepare(&self.path, cancel)
    }
}
fn request(id: u64, root: &Path, path: PathBuf) -> PrepareCapturedRename {
    PrepareCapturedRename {
        id: OperationId(id),
        source_directory: root.into(),
        source: Arc::new(Source {
            path,
            row: TreeRowId(11),
        }),
    }
}

#[tokio::test]
async fn captured_tree_rename_requires_confirmation_and_keeps_original_descendant_after_navigation()
{
    let mut h = Harness::new(MutationKind::Rename);
    let path = h.request.source.path.join("parent/child.txt");
    h.controller
        .prepare_captured_rename(request(1, &h.request.source.path, path.clone()))
        .await;
    let preview = h.preview().await;
    assert_eq!(preview.kind, MutationKind::Rename);
    assert_eq!((preview.files, preview.folders), (1, 0));
    assert_eq!(preview.sample[0].path, path);
    assert!(h.calls.renamed.lock().unwrap().is_empty());
    *h.controller.snapshots.lock().unwrap() = Default::default();
    h.controller
        .confirm(preview.id, Some("renamed.txt".into()))
        .await;
    let done = h.finished().await;
    assert_eq!(done.state, OperationState::Completed, "{done:?}");
    assert_eq!((done.total, done.completed), (1, 1));
    assert_eq!(
        *h.calls.renamed.lock().unwrap(),
        [(path.clone(), "renamed.txt".into())]
    );
    assert_eq!(done.renamed_to, Some(path.with_file_name("renamed.txt")));
}

#[tokio::test]
async fn captured_tree_rename_cancelled_confirmation_and_late_confirm_do_not_mutate() {
    let mut h = Harness::new(MutationKind::Rename);
    h.controller
        .prepare_captured_rename(request(
            1,
            &h.request.source.path,
            h.request.source.path.join("parent/child.txt"),
        ))
        .await;
    h.preview().await;
    h.controller.cancel(OperationId(1));
    assert_eq!(h.finished().await.state, OperationState::Cancelled);
    h.controller
        .confirm(OperationId(1), Some("must-not-rename".into()))
        .await;
    assert!(h.calls.renamed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn captured_tree_rename_parent_scope_is_rejected_before_native_preparation() {
    let mut h = Harness::new(MutationKind::Rename);
    let mut captured = request(1, &h.request.source.path, h.request.source.path.clone());
    captured.source = Arc::new(Source {
        path: h.request.source.path.clone(),
        row: TreeRowId(0),
    });
    h.controller.prepare_captured_rename(captured).await;
    assert_eq!(h.finished().await.state, OperationState::Failed);
    assert_eq!(h.calls.prepared.load(Ordering::Relaxed), 0);
    assert!(h.calls.renamed.lock().unwrap().is_empty());
}
