use super::*;
#[tokio::test]
async fn rename_conflict_native_permission_failure_never_loses_source_or_target() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = fixture();
    let mut run = Run::start(temp.path(), "existing.txt");
    let conflict = run.conflict().await;
    let _deny_delete = fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(temp.path().join("existing.txt"))
        .unwrap();
    run.decide(&conflict, CopyChoice::ReplaceRecoverable { all: false });
    let done = run.finish().await;
    assert_eq!(done.state, OperationState::Failed, "{done:?}");
    assert_eq!(done.completed, 0);
    preserved(temp.path(), b"consented original");
}
