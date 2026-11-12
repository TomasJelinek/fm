//! Actual native pins and overwrite consent for focused descendants.
use super::*;
use std::fs;
fn fixture() -> tempfile::TempDir {
    let root = std::env::temp_dir().join("_build/midnight-tree-rename");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("native-")
        .tempdir_in(fs::canonicalize(root).unwrap())
        .unwrap()
}
fn native() -> Harness {
    Harness::using(
        MutationKind::Rename,
        Some(Arc::new(midnight_platform::LocalFileSystem::new(None))),
    )
}
#[tokio::test]
async fn native_captured_tree_rename_files_and_directories_keep_unselected_siblings() {
    for directory in [false, true] {
        let owned = fixture();
        let parent = owned.path().join("parent");
        fs::create_dir(&parent).unwrap();
        let selected = parent.join("selected 資料");
        if directory {
            fs::create_dir(&selected).unwrap();
            fs::write(selected.join("payload"), b"inside").unwrap();
        } else {
            fs::write(&selected, b"selected").unwrap();
        }
        fs::write(parent.join("keep"), b"not selected").unwrap();
        let mut h = native();
        h.controller
            .prepare_captured_rename(request(1, owned.path(), selected.clone()))
            .await;
        h.preview().await;
        assert!(selected.exists());
        h.controller
            .confirm(OperationId(1), Some("renamed 文档".into()))
            .await;
        let done = h.finished().await;
        assert_eq!(done.state, OperationState::Completed, "{done:?}");
        assert_eq!(done.renamed_to, Some(parent.join("renamed 文档")));
        assert!(!selected.exists());
        assert_eq!(fs::read(parent.join("keep")).unwrap(), b"not selected");
        if directory {
            assert_eq!(
                fs::read(parent.join("renamed 文档/payload")).unwrap(),
                b"inside"
            );
        }
    }
}
#[tokio::test]
async fn native_captured_tree_rename_replaced_source_and_cancelled_preview_preserve_files() {
    for replace in [false, true] {
        let owned = fixture();
        let parent = owned.path().join("parent");
        fs::create_dir(&parent).unwrap();
        let path = parent.join("selected");
        fs::write(&path, b"original").unwrap();
        let mut h = native();
        h.controller
            .prepare_captured_rename(request(1, owned.path(), path.clone()))
            .await;
        h.preview().await;
        if replace {
            fs::rename(&path, parent.join("saved")).unwrap();
            fs::write(&path, b"replacement").unwrap();
        } else {
            h.controller.cancel(OperationId(1));
        }
        h.controller
            .confirm(OperationId(1), Some("must-not-publish".into()))
            .await;
        let done = h.finished().await;
        assert_eq!(
            done.state,
            if replace {
                OperationState::Failed
            } else {
                OperationState::Cancelled
            }
        );
        assert!(!parent.join("must-not-publish").exists());
        assert_eq!(
            fs::read(&path).unwrap(),
            if replace {
                b"replacement".as_slice()
            } else {
                b"original"
            }
        );
    }
}
#[tokio::test]
async fn native_captured_tree_rename_conflict_requires_versioned_consent() {
    let owned = fixture();
    let parent = owned.path().join("parent");
    fs::create_dir(&parent).unwrap();
    let selected = parent.join("selected");
    let existing = parent.join("existing");
    fs::write(&selected, b"selected").unwrap();
    fs::write(&existing, b"original destination").unwrap();
    let mut h = native();
    h.controller
        .prepare_captured_rename(request(1, owned.path(), selected.clone()))
        .await;
    h.preview().await;
    h.controller
        .confirm(OperationId(1), Some("existing".into()))
        .await;
    let conflict = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Event::Operation(OperationEvent::RenameConflict(value))) =
                h.events.recv().await
            {
                break value;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(fs::read(&selected).unwrap(), b"selected");
    assert_eq!(fs::read(&existing).unwrap(), b"original destination");
    h.controller
        .decide_rename(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::ReplaceRecoverable { all: false },
        })
        .await;
    let done = h.finished().await;
    assert_eq!(done.state, OperationState::Completed, "{done:?}");
    assert_eq!(fs::read(&existing).unwrap(), b"selected");
    let recovery = done
        .message
        .lines()
        .skip(1)
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .unwrap();
    assert_eq!(fs::read(recovery).unwrap(), b"original destination");
}
