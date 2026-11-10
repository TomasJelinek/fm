//! Completion notifications must precede later conflicts and never describe partial roots.
use super::*;

#[tokio::test]
async fn completed_move_root_is_reported_only_after_native_source_relocation() {
    let (_temp, source) = moves::protected_fixture();
    let destination = source.join("target");
    fs::create_dir(&destination).unwrap();
    let first = source.join("first");
    let second = source.join("second");
    fs::write(&first, b"verified move").unwrap();
    fs::write(&second, b"retain").unwrap();
    fs::write(destination.join("second"), b"collision").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue_move(request(1, &[first.clone(), second.clone()], &destination))
        .await;
    let mut completed = Vec::new();
    let conflict = loop {
        match h.next().await {
            CopyEvent::RootCompleted {
                source: CompletedCopySource::Native(path),
                ..
            } => {
                assert!(
                    !first.exists(),
                    "Move feedback arrived before source removal"
                );
                assert_eq!(
                    fs::read(destination.join("first")).unwrap(),
                    b"verified move"
                );
                completed.push(path);
            }
            CopyEvent::Conflict(conflict) => break conflict,
            CopyEvent::Progress(p) => assert!(!p.state.is_terminal(), "{p:?}"),
            _ => panic!("wrong provider"),
        }
    };
    assert_eq!(completed, vec![first]);
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::Cancel,
        })
        .await;
    loop {
        match h.next().await {
            CopyEvent::RootCompleted { .. } => panic!("Cancelled move reported complete"),
            CopyEvent::Progress(p) if p.state.is_terminal() => {
                assert_eq!(p.state, CopyOperationState::Cancelled);
                assert_eq!(p.source_files_removed, 0);
                assert_eq!(p.native_entries_moved, 1);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(fs::read(second).unwrap(), b"retain");
    h.close().await;
}

#[tokio::test]
async fn partially_skipped_folder_is_not_reported_complete() {
    let temp = fixture();
    let folder = temp.path().join("folder");
    let destination = temp.path().join("target");
    fs::create_dir_all(destination.join("folder")).unwrap();
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("child"), b"source").unwrap();
    fs::write(destination.join("folder/child"), b"target").unwrap();
    let last = temp.path().join("last");
    fs::write(&last, b"last").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, &[folder, last.clone()], &destination))
        .await;
    let mut completed = Vec::new();
    loop {
        match h.next().await {
            CopyEvent::RootCompleted {
                source: CompletedCopySource::Native(path),
                ..
            } => completed.push(path),
            CopyEvent::Conflict(c) => {
                h.controller
                    .decide(CopyDecision {
                        id: c.id,
                        version: c.version,
                        choice: if c.can_merge {
                            CopyChoice::Merge { all: false }
                        } else {
                            CopyChoice::Skip
                        },
                    })
                    .await
            }
            CopyEvent::Progress(p) if p.state.is_terminal() => {
                assert_eq!(p.skipped, 1);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(completed, vec![last]);
    assert_eq!(
        fs::read(destination.join("folder/child")).unwrap(),
        b"target"
    );
    h.close().await;
}

#[test]
fn cancelled_unfinished_root_does_not_emit_completion() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("target");
    fs::write(&source, b"original").unwrap();
    fs::create_dir(&destination).unwrap();
    let request = request(1, std::slice::from_ref(&source), &destination);
    let status = Arc::new(Mutex::new(initial(&request.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut job = worker::Job::prepare(request, &service, cancel.clone(), status).unwrap();
    assert!(matches!(job.step(), worker::Step::Continue));
    cancel.cancel();
    for _ in 0..100 {
        let step = job.step();
        assert!(job.take_completed_root().is_none());
        if matches!(step, worker::Step::Done) {
            assert_eq!(job.finish().state, CopyOperationState::Cancelled);
            assert!(!destination.join("source").exists());
            return;
        }
    }
    panic!("worker did not stop");
}

#[tokio::test]
async fn completed_copy_roots_are_reported_before_the_next_conflict() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("target");
    fs::create_dir_all(source.join("folder")).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("first 文档"), b"verified bytes").unwrap();
    fs::write(source.join("folder/child"), b"nested bytes").unwrap();
    fs::write(source.join("last"), b"skip me").unwrap();
    fs::write(destination.join("last"), b"keep me").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(
            1,
            &[
                source.join("first 文档"),
                source.join("folder"),
                source.join("last"),
            ],
            &destination,
        ))
        .await;
    let mut completed = Vec::new();
    let conflict = loop {
        match h.next().await {
            CopyEvent::RootCompleted {
                id,
                source: CompletedCopySource::Native(path),
            } => {
                assert_eq!(id, OperationId(1));
                completed.push(path);
            }
            CopyEvent::Conflict(conflict) => break conflict,
            CopyEvent::Progress(p) => assert!(!p.state.is_terminal(), "{p:?}"),
            _ => panic!("native copy reported a virtual identity"),
        }
    };
    assert_eq!(
        completed,
        vec![source.join("first 文档"), source.join("folder")]
    );
    assert_eq!(
        fs::read(destination.join("first 文档")).unwrap(),
        b"verified bytes"
    );
    assert_eq!(
        fs::read(destination.join("folder/child")).unwrap(),
        b"nested bytes"
    );
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::Skip,
        })
        .await;
    loop {
        match h.next().await {
            CopyEvent::RootCompleted { .. } => panic!("Skipped root reported complete"),
            CopyEvent::Progress(p) if p.state.is_terminal() => {
                assert_eq!(p.skipped, 1);
                break;
            }
            _ => {}
        }
    }
    h.close().await;
}
