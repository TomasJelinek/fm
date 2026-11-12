//! Real native fixtures exercise authority revocation independently of navigation.
use midnight_core::*;
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;

async fn event(receive: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(10), receive.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn search(
    send: &mpsc::Sender<Command>,
    receive: &mut mpsc::Receiver<Event>,
    root: &Path,
    generation: u64,
) -> Vec<Arc<FindResult>> {
    send.send(Command::StartFind(Box::new(FindRequest::new(
        FindSessionId(1),
        FindGeneration(generation),
        vec![root.to_owned()],
    ))))
    .await
    .unwrap();
    let mut results = Vec::new();
    loop {
        if let Event::Find(update) = event(receive).await {
            if update.generation != FindGeneration(generation) {
                continue;
            }
            match update.update {
                FindUpdate::Batch(batch) => results.extend(batch.iter().cloned()),
                FindUpdate::Finished(_) => return results,
                _ => {}
            }
        }
    }
}
async fn capture(
    send: &mpsc::Sender<Command>,
    receive: &mut mpsc::Receiver<Event>,
    result: FindResultId,
    generation: u64,
    token: u64,
) -> Result<Arc<dyn CapturedFindFile>, FsError> {
    send.send(Command::CaptureFindFile(FindValidationRequest {
        session: FindSessionId(1),
        generation: FindGeneration(generation),
        result,
        token,
    }))
    .await
    .unwrap();
    loop {
        if let Event::FindFileCaptured(result) = event(receive).await
            && result.request.token == token
        {
            return result.result;
        }
    }
}
fn engine() -> (
    mpsc::Sender<Command>,
    mpsc::Receiver<Event>,
    tokio::task::JoinHandle<()>,
) {
    let service = Arc::new(LocalFileSystem::new(None));
    let (send, commands) = mpsc::channel(32);
    let (events, receive) = mpsc::channel(32);
    let mut engine = Engine::new(Engine::lifetime().unwrap(), service.clone(), events);
    engine.configure_find().unwrap().install(service);
    (send, receive, tokio::spawn(engine.run(commands)))
}

#[tokio::test]
async fn captures_validate_changes_and_reject_directory_or_unknown_identity() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("captured 文档.txt");
    std::fs::write(&path, b"first").unwrap();
    std::fs::create_dir(root.path().join("folder")).unwrap();
    let (send, mut receive, engine) = engine();
    let results = search(&send, &mut receive, root.path(), 1).await;
    let file = results
        .iter()
        .find(|r| r.entry.kind == EntryKind::File)
        .unwrap();
    let folder = results
        .iter()
        .find(|r| r.entry.kind == EntryKind::Directory)
        .unwrap();
    assert!(capture(&send, &mut receive, folder.id, 1, 1).await.is_err());
    assert!(
        capture(&send, &mut receive, FindResultId(900), 1, 2)
            .await
            .is_err()
    );
    let source = capture(&send, &mut receive, file.id, 1, 3).await.unwrap();
    assert_eq!(source.entry().path, path);
    source.validate(&Cancellation::default()).unwrap();
    std::fs::write(&path, b"changed after action capture").unwrap();
    assert!(source.validate(&Cancellation::default()).is_err());
    send.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test]
async fn stop_keeps_file_actions_but_restart_and_close_revoke_captures() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"contents").unwrap();
    let (send, mut receive, engine) = engine();
    let results = search(&send, &mut receive, root.path(), 1).await;
    send.send(Command::CancelFind {
        session: FindSessionId(1),
        generation: FindGeneration(1),
    })
    .await
    .unwrap();
    let source = capture(&send, &mut receive, results[0].id, 1, 1)
        .await
        .unwrap();
    source.validate(&Cancellation::default()).unwrap();
    let replaced = search(&send, &mut receive, root.path(), 2).await;
    assert!(source.validate(&Cancellation::default()).is_err());
    let current = capture(&send, &mut receive, replaced[0].id, 2, 2)
        .await
        .unwrap();
    current.validate(&Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(current.validate(&cancel).is_err());
    send.send(Command::CloseFind(FindSessionId(1)))
        .await
        .unwrap();
    // A subsequent capture rejection acknowledges processing of the earlier Close.
    assert!(
        capture(&send, &mut receive, replaced[0].id, 2, 3)
            .await
            .is_err()
    );
    assert!(current.validate(&Cancellation::default()).is_err());
    send.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test]
async fn live_capture_limit_is_independent_of_result_validation_and_shutdown() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"contents").unwrap();
    let (send, mut receive, engine) = engine();
    let result = search(&send, &mut receive, root.path(), 1).await.remove(0);
    let mut retained = Vec::new();
    for token in 1..=16 {
        retained.push(
            capture(&send, &mut receive, result.id, 1, token)
                .await
                .unwrap(),
        );
    }
    assert!(
        capture(&send, &mut receive, result.id, 1, 17)
            .await
            .is_err()
    );
    send.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
    assert!(retained[0].validate(&Cancellation::default()).is_err());
    drop(retained); // Retirement remains live after the engine and runtime's task ownership.
}

#[test]
fn capture_provider_final_drop_survives_runtime_shutdown_off_caller_thread() {
    struct Tracked {
        native: LocalFileSystem,
        dropped: Option<std::sync::mpsc::Sender<std::thread::ThreadId>>,
    }
    impl FindService for Tracked {
        fn open_find_source(
            &self,
            path: &Path,
            cancel: &Cancellation,
        ) -> Result<Box<dyn FindSource>, FsError> {
            self.native.open_find_source(path, cancel)
        }
    }
    impl Drop for Tracked {
        fn drop(&mut self) {
            if let Some(sender) = self.dropped.take() {
                let _ = sender.send(std::thread::current().id());
            }
        }
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"contents").unwrap();
    let (dropped, observe) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = runtime.block_on(async {
        let (send, commands) = mpsc::channel(16);
        let (events, mut receive) = mpsc::channel(16);
        let mut engine = Engine::new(
            Engine::lifetime().unwrap(),
            Arc::new(LocalFileSystem::new(None)),
            events,
        );
        engine.configure_find().unwrap().install(Arc::new(Tracked {
            native: LocalFileSystem::new(None),
            dropped: Some(dropped),
        }));
        let running = tokio::spawn(engine.run(commands));
        let result = search(&send, &mut receive, root.path(), 1).await.remove(0);
        let source = capture(&send, &mut receive, result.id, 1, 1).await.unwrap();
        send.send(Command::Shutdown).await.unwrap();
        running.await.unwrap();
        source
    });
    drop(runtime);
    assert!(
        observe.try_recv().is_err(),
        "capture must retain its provider"
    );
    drop(source);
    assert_ne!(
        observe.recv_timeout(Duration::from_secs(5)).unwrap(),
        std::thread::current().id()
    );
}
