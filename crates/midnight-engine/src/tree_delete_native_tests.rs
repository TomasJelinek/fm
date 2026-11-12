//! Actual native preparation/deletion stays within this test's generated fixture.
use super::*;
use std::fs;

use crate::native_test_links as links;

fn native() -> Harness {
    Harness::using(
        MutationKind::Delete(DeleteMode::Permanent),
        Some(Arc::new(midnight_platform::LocalFileSystem::new(None))),
    )
}
fn captured(id: u64, root: &Path, paths: Vec<PathBuf>) -> PrepareCapturedDelete {
    PrepareCapturedDelete {
        id: OperationId(id),
        source_directory: root.into(),
        sources: Arc::new(Sources {
            paths,
            mode: DeleteMode::Permanent,
            dropped: None,
        }),
    }
}
fn fixture() -> tempfile::TempDir {
    let root = std::env::temp_dir().join("midnight-tree-delete/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("native-")
        .tempdir_in(root)
        .unwrap()
}

#[tokio::test]
async fn native_tree_delete_selected_nested_siblings_use_the_existing_confirmed_worker() {
    let owned = fixture();
    let first = owned.path().join("first/selected 資料");
    let second = owned.path().join("second/selected 文档");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(first.join("payload"), b"first").unwrap();
    fs::write(second.join("payload"), b"second").unwrap();
    let keep = owned.path().join("keep.txt");
    fs::write(&keep, b"not selected").unwrap();
    let mut h = native();
    h.controller
        .prepare_captured_delete(captured(
            1,
            owned.path(),
            vec![first.clone(), second.clone()],
        ))
        .await;
    let preview = h.preview().await;
    assert_eq!((preview.files, preview.folders), (0, 2));
    assert!(first.join("payload").exists());
    assert!(second.join("payload").exists());
    h.controller.confirm(OperationId(1), None).await;
    let done = h.finished().await;
    assert_eq!(done.state, OperationState::Completed, "{done:?}");
    assert_eq!((done.total, done.completed), (2, 2));
    assert!(!first.exists() && !second.exists());
    assert!(owned.path().join("first").is_dir());
    assert!(owned.path().join("second").is_dir());
    assert_eq!(fs::read(keep).unwrap(), b"not selected");
}

#[tokio::test]
async fn native_tree_delete_cancelled_confirmation_preserves_all_nested_contents() {
    let owned = fixture();
    let directory = owned.path().join("parent/selected/child");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("payload"), b"preserved").unwrap();
    let mut h = native();
    h.controller
        .prepare_captured_delete(captured(1, owned.path(), vec![directory.clone()]))
        .await;
    h.preview().await;
    h.controller.cancel(OperationId(1));
    assert_eq!(h.finished().await.state, OperationState::Cancelled);
    h.controller.confirm(OperationId(1), None).await;
    assert_eq!(fs::read(directory.join("payload")).unwrap(), b"preserved");
}

#[tokio::test]
async fn native_tree_delete_selected_directory_link_preserves_its_target() {
    let owned = fixture();
    let target = owned.path().join("unselected target");
    let link = owned.path().join("selected link");
    fs::create_dir(&target).unwrap();
    fs::write(
        target.join("payload"),
        b"never follow selected directory links",
    )
    .unwrap();
    links::create(&link, &target);
    let mut h = native();
    h.controller
        .prepare_captured_delete(captured(1, owned.path(), vec![link.clone()]))
        .await;
    let preview = h.preview().await;
    assert!(matches!(
        preview.sample[0].kind,
        EntryKind::DirectoryLink | EntryKind::Symlink
    ));
    h.controller.confirm(OperationId(1), None).await;
    let done = h.finished().await;
    assert_eq!(done.state, OperationState::Completed, "{done:?}");
    assert!(fs::symlink_metadata(link).is_err());
    assert_eq!(
        fs::read(target.join("payload")).unwrap(),
        b"never follow selected directory links"
    );
}

#[tokio::test]
async fn native_tree_delete_rejects_replaced_selected_name_after_confirmation_capture() {
    let owned = fixture();
    let original = owned.path().join("parent/selected.txt");
    let moved = owned.path().join("parent/moved-original.txt");
    fs::create_dir(original.parent().unwrap()).unwrap();
    fs::write(&original, b"captured original").unwrap();
    let mut h = native();
    h.controller
        .prepare_captured_delete(captured(1, owned.path(), vec![original.clone()]))
        .await;
    h.preview().await;
    fs::rename(&original, &moved).unwrap();
    fs::write(&original, b"replacement must survive").unwrap();
    h.controller.confirm(OperationId(1), None).await;
    let done = h.finished().await;
    assert_eq!(done.state, OperationState::Failed, "{done:?}");
    assert_eq!(fs::read(original).unwrap(), b"replacement must survive");
    assert_eq!(fs::read(moved).unwrap(), b"captured original");
}

#[tokio::test]
async fn native_tree_delete_never_follows_a_directory_replaced_by_a_link_after_preview() {
    let owned = fixture();
    let selected = owned.path().join("selected");
    let child = selected.join("child");
    let saved = owned.path().join("moved-out-original-child");
    let target = owned.path().join("new-link-target");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(child.join("payload"), b"moved outside selected tree").unwrap();
    fs::write(target.join("payload"), b"link target is not selected").unwrap();
    let mut h = native();
    h.controller
        .prepare_captured_delete(captured(1, owned.path(), vec![selected.clone()]))
        .await;
    h.preview().await;
    fs::rename(&child, &saved).unwrap();
    links::create(&child, &target);
    h.controller.confirm(OperationId(1), None).await;
    let done = h.finished().await;
    assert_eq!(done.state, OperationState::Completed, "{done:?}");
    assert!(!selected.exists());
    assert_eq!(
        fs::read(saved.join("payload")).unwrap(),
        b"moved outside selected tree"
    );
    assert_eq!(
        fs::read(target.join("payload")).unwrap(),
        b"link target is not selected"
    );
}
