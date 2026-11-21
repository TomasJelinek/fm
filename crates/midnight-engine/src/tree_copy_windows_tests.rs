//! Tree-captured Copy roots on Windows: a root prepared without its read guard (its
//! provider opens it at first use) takes it before the job stages anything for it.
use super::*;
use midnight_core::{
    TransferStartPolicy, TransferVerification, TreeRequestId, TreeTransferRequest,
};
use std::os::windows::fs::OpenOptionsExt;

async fn failure(h: &mut Harness, id: u64) -> Arc<CopyFailure> {
    loop {
        match h.next().await {
            CopyEvent::Failure(failure) if failure.id == OperationId(id) => return failure,
            CopyEvent::Progress(progress) if progress.id == OperationId(id) => {
                assert!(
                    !progress.state.is_terminal(),
                    "expected a failure: {progress:?}"
                );
            }
            _ => {}
        }
    }
}

/// F5 in the tree on a file another program holds without sharing fails as it does from a
/// panel: before the job creates the file's stage in the destination, so nothing of it is
/// there while its Retry question waits, and Retry copies it once the file is released.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tree_copy_of_a_held_file_asks_before_it_stages_anything() {
    let (folder, mut session, _registry) = crate::tree_copy::tests::fixture(false);
    fs::write(folder.path().join("file-1"), b"held file original").unwrap();
    fs::write(folder.path().join("file-2"), b"free file").unwrap();
    let publication = session.publication().unwrap();
    session.commit(&publication.view);
    let capture = session
        .capture_transfer(
            &TreeTransferRequest {
                intent: TransferIntent::Copy,
                view: publication.view.clone(),
                request: TreeRequestId(1),
                selection_revision: publication.selection_revision,
                selection: publication.selection.clone(),
                fallback: None,
                cancel: Cancellation::default(),
            },
            &Arc::new(tokio::sync::Semaphore::new(4)),
            &Cancellation::default(),
        )
        .unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(folder.path().join("file-1"))
        .unwrap();
    let destination = fixture();
    let mut h = Harness::native();
    h.controller
        .enqueue_captured(CapturedTransferRequest {
            start_policy: TransferStartPolicy::Immediate,
            verification: TransferVerification::default(),
            id: OperationId(1),
            sources: capture.sources,
            destination: destination.path().to_owned(),
            metadata: TransferMetadataPolicy::ContentOnly,
        })
        .await;
    let refused = failure(&mut h, 1).await;
    assert!(refused.can_retry, "{refused:?}");
    assert!(
        refused.error.message.ends_with("(os error 32)"),
        "{refused:?}"
    );
    assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);
    h.controller
        .decide(CopyDecision {
            id: refused.id,
            version: refused.version,
            choice: CopyChoice::Retry,
        })
        .await;
    let still = failure(&mut h, 1).await;
    assert!(still.version > refused.version);
    assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);
    drop(held);
    h.controller
        .decide(CopyDecision {
            id: still.id,
            version: still.version,
            choice: CopyChoice::Retry,
        })
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(
        fs::read(destination.path().join("file-1")).unwrap(),
        b"held file original"
    );
    assert_eq!(
        fs::read(destination.path().join("file-2")).unwrap(),
        b"free file"
    );
    h.close().await;
    drop(session);
}
