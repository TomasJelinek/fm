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
    send: &midnight_engine::CommandSender,
    receive: &mut mpsc::Receiver<Event>,
    root: &Path,
    generation: u64,
) -> Vec<Arc<FindResult>> {
    search_request(
        send,
        receive,
        FindRequest::new(
            FindSessionId(1),
            FindGeneration(generation),
            vec![root.to_owned()],
        ),
    )
    .await
    .0
}
async fn search_request(
    send: &midnight_engine::CommandSender,
    receive: &mut mpsc::Receiver<Event>,
    request: FindRequest,
) -> (Vec<Arc<FindResult>>, FindSummary) {
    let owner = (request.session, request.generation);
    send.send(Command::StartFind(Box::new(request)))
        .await
        .unwrap();
    let mut results = Vec::new();
    loop {
        if let Event::Find(update) = event(receive).await {
            if (update.session, update.generation) != owner {
                continue;
            }
            match update.update {
                FindUpdate::Batch(batch) => results.extend(batch.iter().cloned()),
                FindUpdate::Finished(summary) => return (results, summary),
                _ => {}
            }
        }
    }
}
fn same_request(actual: FindValidationRequest, expected: FindValidationRequest) -> bool {
    actual.session == expected.session
        && actual.generation == expected.generation
        && actual.result == expected.result
        && actual.token == expected.token
}
async fn validate(
    send: &midnight_engine::CommandSender,
    receive: &mut mpsc::Receiver<Event>,
    request: FindValidationRequest,
) -> Result<Box<FindEntry>, FsError> {
    send.send(Command::ValidateFindResult(request))
        .await
        .unwrap();
    loop {
        if let Event::FindResultValidated(result) = event(receive).await
            && same_request(result.request, request)
        {
            return result.result;
        }
    }
}
async fn capture(
    send: &midnight_engine::CommandSender,
    receive: &mut mpsc::Receiver<Event>,
    result: FindResultId,
    generation: u64,
    token: u64,
) -> Result<Arc<dyn CapturedFindFile>, FsError> {
    let request = FindValidationRequest {
        session: FindSessionId(1),
        generation: FindGeneration(generation),
        result,
        token,
    };
    send.send(Command::CaptureFindFile(request)).await.unwrap();
    loop {
        if let Event::FindFileCaptured(result) = event(receive).await
            && same_request(result.request, request)
        {
            return result.result;
        }
    }
}
fn engine() -> (
    midnight_engine::CommandSender,
    mpsc::Receiver<Event>,
    tokio::task::JoinHandle<()>,
) {
    let service = Arc::new(LocalFileSystem::new(None));

    let (events, receive) = mpsc::channel(32);
    let mut engine = Engine::new(Engine::lifetime().unwrap(), service.clone(), events);
    let (send, commands) = engine.command_channel(32).unwrap();
    engine.configure_find().unwrap().install(service);
    (send, receive, tokio::spawn(engine.run(commands)))
}

/// Holds result validation of the `blocked` file inside its native open,
/// like an unresponsive network share, until the test releases it.
struct BlockingValidation {
    native: Arc<LocalFileSystem>,
    armed: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    released: (std::sync::Mutex<bool>, std::sync::Condvar),
}
impl BlockingValidation {
    fn release(&self) {
        *self.released.0.lock().unwrap() = true;
        self.released.1.notify_all();
    }
}
impl FindService for BlockingValidation {
    fn open_find_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn FindSource>, FsError> {
        if self.armed.load(std::sync::atomic::Ordering::SeqCst)
            && path.file_name() == Some(std::ffi::OsStr::new("blocked"))
        {
            self.entered.notify_one();
            let released = self.released.0.lock().unwrap();
            let _ = self
                .released
                .1
                .wait_timeout_while(released, Duration::from_secs(10), |released| !*released)
                .unwrap();
        }
        self.native.open_find_source(path, cancel)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_blocked_result_validation_does_not_stall_another_validation() {
    let root = tempfile::tempdir().unwrap();
    for name in ["blocked", "free"] {
        std::fs::write(root.path().join(name), name).unwrap();
    }
    let native = Arc::new(LocalFileSystem::new(None));
    let service = Arc::new(BlockingValidation {
        native: native.clone(),
        armed: Default::default(),
        entered: Default::default(),
        released: Default::default(),
    });
    let (events, mut receive) = mpsc::channel(32);
    let mut engine = Engine::new(Engine::lifetime().unwrap(), native, events);
    let (send, commands) = engine.command_channel(32).unwrap();
    engine.configure_find().unwrap().install(service.clone());
    let engine = tokio::spawn(engine.run(commands));
    let results = search(&send, &mut receive, root.path(), 1).await;
    let id = |name: &str| {
        results
            .iter()
            .find(|row| row.entry.path.file_name() == Some(std::ffi::OsStr::new(name)))
            .unwrap()
            .id
    };
    let request = |result, token| FindValidationRequest {
        session: FindSessionId(1),
        generation: FindGeneration(1),
        result,
        token,
    };
    service
        .armed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    send.send(Command::ValidateFindResult(request(id("blocked"), 1)))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), service.entered.notified())
        .await
        .expect("the first validation reaches its native open");
    // The second request completes while the first is still blocked.
    let free = tokio::time::timeout(
        Duration::from_secs(5),
        validate(&send, &mut receive, request(id("free"), 2)),
    )
    .await
    .expect("another validation must not wait for the blocked native open");
    assert!(free.is_ok());
    service.release();
    assert!(
        validate(&send, &mut receive, request(id("blocked"), 3))
            .await
            .is_ok()
    );
    send.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
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
        let (events, mut receive) = mpsc::channel(16);
        let mut engine = Engine::new(
            Engine::lifetime().unwrap(),
            Arc::new(LocalFileSystem::new(None)),
            events,
        );
        let (send, commands) = engine.command_channel(16).unwrap();
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

#[tokio::test]
async fn nested_immutable_find_result_keeps_revision_through_repeated_actions() {
    let fixture_parent =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp/find-result-revision/engine");
    std::fs::create_dir_all(&fixture_parent).unwrap();
    let fixture = tempfile::tempdir_in(std::fs::canonicalize(fixture_parent).unwrap()).unwrap();
    let nested = fixture.path().join("nested 文档").join("include");
    std::fs::create_dir_all(&nested).unwrap();
    let target = nested.join("DynamicMatrixCacheLimit.h");
    std::fs::write(&target, vec![b'x'; 2670]).unwrap();
    let modified = std::time::UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_700);
    std::fs::File::options()
        .write(true)
        .open(&target)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let (send, mut receive, running) = engine();
    let outcome = tokio::time::timeout(Duration::from_secs(30), async {
        let mut query = FindRequest::new(
            FindSessionId(1),
            FindGeneration(1),
            vec![fixture.path().to_owned()],
        );
        query.masks = "*cache*".into();
        let (results, summary) = search_request(&send, &mut receive, query).await;
        assert_eq!(summary.completion, FindCompletion::Completed);
        let result = results
            .iter()
            .find(|result| result.entry.path == target)
            .unwrap();
        assert_eq!(result.entry.metadata.size, Some(2670));
        assert_eq!(result.entry.metadata.modified, Some(modified));
        for token in 1..=4 {
            let request = FindValidationRequest {
                session: FindSessionId(1),
                generation: FindGeneration(1),
                result: result.id,
                token,
            };
            let validated = validate(&send, &mut receive, request).await.unwrap();
            assert_eq!(validated.revision, result.entry.revision);
            assert!(validated.matches_capture(&result.entry));
            let source = capture(&send, &mut receive, result.id, 1, token)
                .await
                .unwrap();
            assert_eq!(source.entry().revision, result.entry.revision);
            assert!(source.entry().matches_capture(&result.entry));
            source.validate(&Cancellation::default()).unwrap();
        }
    })
    .await;
    let stopped = send.send(Command::Shutdown).await;
    let joined = tokio::time::timeout(Duration::from_secs(10), running).await;
    stopped.unwrap();
    joined.unwrap().unwrap();
    outcome.unwrap();
}
