use midnight_core::*;
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{fs, path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;

async fn next_find(events: &mut mpsc::Receiver<Event>) -> FindEvent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::Find(event) = events.recv().await.expect("engine event stream") {
                return *event;
            }
        }
    })
    .await
    .expect("Find event timeout")
}
fn request(path: &Path, session: u64, generation: u64) -> FindRequest {
    FindRequest::new(
        FindSessionId(session),
        FindGeneration(generation),
        vec![path.to_owned()],
    )
}

#[tokio::test]
async fn sorted_views_keep_discovery_validation_authority_and_latest_revision() {
    struct ReverseLocale;
    impl LocaleService for ReverseLocale {
        fn number_locale(&self) -> Result<NumberLocale, FsError> {
            Ok(NumberLocale::default())
        }
        fn collation_key(&self, value: &std::ffi::OsStr) -> Result<Option<Vec<u8>>, FsError> {
            Ok(value.to_str().map(|s| {
                s.bytes()
                    .map(|byte| 255 - byte.to_ascii_lowercase())
                    .collect()
            }))
        }
    }
    let temporary = tempfile::tempdir().unwrap();
    for (name, bytes) in [("A.txt", 3), ("z.txt", 1), ("b.md", 2)] {
        fs::write(temporary.path().join(name), vec![42; bytes]).unwrap();
    }
    let service = Arc::new(LocalFileSystem::new(None));
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            service.clone(),
            events,
        )
        .with_find(service)
        .with_locale(Arc::new(ReverseLocale))
        .run(commands),
    );
    send.send(Command::StartFind(Box::new(request(
        temporary.path(),
        1,
        1,
    ))))
    .await
    .unwrap();
    let mut authority = Vec::new();
    let mut finished = false;
    let mut latest = None;
    let order = loop {
        let event = next_find(&mut receive).await;
        match event.update {
            FindUpdate::Batch(batch) => authority.extend(batch.iter().cloned()),
            FindUpdate::Finished(_) => finished = true,
            FindUpdate::Sorted(order) if order.ids.len() == 3 => latest = Some(order),
            _ => {}
        }
        if finished && let Some(order) = latest.take() {
            break order;
        }
    };
    let names = |order: &FindOrder| {
        order
            .ids
            .iter()
            .map(|id| {
                authority[(id.0 - 1) as usize]
                    .entry
                    .path
                    .file_name()
                    .unwrap()
                    .to_owned()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&order), ["z.txt", "b.md", "A.txt"]);
    for (revision, descending) in [(1, true), (2, false)] {
        send.send(Command::SortFind(FindSortRequest {
            session: FindSessionId(1),
            generation: FindGeneration(1),
            revision,
            sort: FindSortSpec {
                column: FindSortColumn::Size,
                descending,
            },
        }))
        .await
        .unwrap();
    }
    send.send(Command::SortFind(FindSortRequest {
        session: FindSessionId(1),
        generation: FindGeneration(0),
        revision: 99,
        sort: FindSortSpec {
            column: FindSortColumn::Modified,
            descending: true,
        },
    }))
    .await
    .unwrap();
    let order = loop {
        let event = next_find(&mut receive).await;
        if let FindUpdate::Sorted(order) = event.update {
            assert_ne!(order.revision, 99);
            if order.revision == 2 {
                break order;
            }
        }
    };
    assert_eq!(names(&order), ["z.txt", "b.md", "A.txt"]);
    let result = &authority[0];
    send.send(Command::ValidateFindResult(FindValidationRequest {
        session: FindSessionId(1),
        generation: FindGeneration(1),
        result: result.id,
        token: 55,
    }))
    .await
    .unwrap();
    loop {
        if let Event::FindResultValidated(event) = receive.recv().await.unwrap() {
            assert_eq!(event.result.unwrap().path, result.entry.path);
            break;
        }
    }
    drop(send);
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn native_find_streams_and_revalidates_retained_results_without_panel_navigation() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("result 文档.txt");
    fs::write(&path, "original").unwrap();
    let service = Arc::new(LocalFileSystem::new(None));
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            service.clone(),
            events,
        )
        .with_find(service)
        .run(commands),
    );
    send.send(Command::StartFind(Box::new(request(
        temporary.path(),
        1,
        1,
    ))))
    .await
    .unwrap();
    let mut result = None;
    loop {
        let event = next_find(&mut receive).await;
        assert_eq!(
            (event.session, event.generation),
            (FindSessionId(1), FindGeneration(1))
        );
        match event.update {
            FindUpdate::Batch(batch) => {
                assert_eq!(batch.len(), 1);
                result = Some(batch[0].clone());
            }
            FindUpdate::Finished(summary) => {
                assert_eq!(summary.completion, FindCompletion::Completed);
                break;
            }
            _ => {}
        }
    }
    let result = result.unwrap();
    assert_eq!(result.entry.path, path);
    let validation = FindValidationRequest {
        session: FindSessionId(1),
        generation: FindGeneration(1),
        result: result.id,
        token: 9,
    };
    send.send(Command::ValidateFindResult(validation))
        .await
        .unwrap();
    loop {
        if let Event::FindResultValidated(event) = receive.recv().await.unwrap() {
            assert_eq!(event.request.token, 9);
            assert!(event.result.is_ok());
            break;
        }
    }
    fs::write(&path, "replacement contents with another size").unwrap();
    send.send(Command::ValidateFindResult(FindValidationRequest {
        token: 10,
        ..validation
    }))
    .await
    .unwrap();
    loop {
        if let Event::FindResultValidated(event) = receive.recv().await.unwrap() {
            assert_eq!(event.request.token, 10);
            assert!(event.result.is_err());
            break;
        }
    }
    send.send(Command::CloseFind(FindSessionId(1)))
        .await
        .unwrap();
    send.send(Command::ValidateFindResult(FindValidationRequest {
        token: 11,
        ..validation
    }))
    .await
    .unwrap();
    loop {
        if let Event::FindResultValidated(event) = receive.recv().await.unwrap() {
            assert!(event.result.is_err());
            break;
        }
    }
    drop(send);
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cancellation_under_backpressure_finishes_and_superseding_queries_reject_old_batches() {
    let temporary = tempfile::tempdir().unwrap();
    for index in 0..200 {
        fs::write(temporary.path().join(format!("result-{index}.txt")), "data").unwrap();
    }
    let service = Arc::new(LocalFileSystem::new(None));
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(1);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            service.clone(),
            events,
        )
        .with_find(service)
        .run(commands),
    );
    send.send(Command::StartFind(Box::new(request(
        temporary.path(),
        1,
        1,
    ))))
    .await
    .unwrap();
    send.send(Command::CancelFind {
        session: FindSessionId(1),
        generation: FindGeneration(1),
    })
    .await
    .unwrap();
    loop {
        let event = next_find(&mut receive).await;
        if let FindUpdate::Finished(summary) = event.update {
            assert_eq!(summary.completion, FindCompletion::Cancelled);
            break;
        }
    }
    send.send(Command::StartFind(Box::new(request(
        temporary.path(),
        1,
        2,
    ))))
    .await
    .unwrap();
    let first = next_find(&mut receive).await;
    assert_eq!(first.generation, FindGeneration(2));
    let mut newer = request(temporary.path(), 1, 3);
    newer.masks = "no matching names".into();
    send.send(Command::StartFind(Box::new(newer)))
        .await
        .unwrap();
    let mut started_new = false;
    loop {
        let event = next_find(&mut receive).await;
        if event.generation == FindGeneration(3) {
            started_new = true;
        }
        if started_new {
            assert_eq!(event.generation, FindGeneration(3));
        }
        if let FindUpdate::Finished(summary) = event.update
            && event.generation == FindGeneration(3)
        {
            assert_eq!(summary.completion, FindCompletion::Completed);
            assert_eq!(summary.progress.results, 0);
            break;
        }
    }
    drop(send);
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn unavailable_find_and_reused_closed_session_ids_fail_explicitly() {
    let temporary = tempfile::tempdir().unwrap();
    let service = Arc::new(LocalFileSystem::new(None));
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            service,
            events,
        )
        .run(commands),
    );
    send.send(Command::StartFind(Box::new(request(
        temporary.path(),
        1,
        1,
    ))))
    .await
    .unwrap();
    loop {
        if let Event::FindRejected(event) = receive.recv().await.unwrap() {
            assert_eq!(event.error.kind, std::io::ErrorKind::Unsupported);
            break;
        }
    }
    drop(send);
    engine.await.unwrap();
    let service = Arc::new(LocalFileSystem::new(None));
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            service.clone(),
            events,
        )
        .with_find(service)
        .run(commands),
    );
    send.send(Command::StartFind(Box::new(request(
        temporary.path(),
        1,
        1,
    ))))
    .await
    .unwrap();
    send.send(Command::CloseFind(FindSessionId(1)))
        .await
        .unwrap();
    send.send(Command::StartFind(Box::new(request(
        temporary.path(),
        1,
        1,
    ))))
    .await
    .unwrap();
    loop {
        if let Event::FindRejected(event) = receive.recv().await.unwrap() {
            assert_eq!(event.error.kind, std::io::ErrorKind::InvalidInput);
            break;
        }
    }
    drop(send);
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn pending_find_can_cancel_while_two_native_calls_are_blocked_and_session_admission_is_bounded()
 {
    use std::sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    struct Gate {
        open: Mutex<bool>,
        changed: Condvar,
        entered: AtomicUsize,
    }
    struct Release(Arc<Gate>);
    impl Drop for Release {
        fn drop(&mut self) {
            *self.0.open.lock().unwrap() = true;
            self.0.changed.notify_all();
        }
    }
    struct Service {
        native: Arc<LocalFileSystem>,
        gate: Arc<Gate>,
    }
    impl FindService for Service {
        fn open_find_source(
            &self,
            path: &Path,
            cancel: &Cancellation,
        ) -> Result<Box<dyn FindSource>, FsError> {
            if path.file_name().is_some_and(|name| name == "blocked") {
                self.gate.entered.fetch_add(1, Ordering::SeqCst);
                let mut open = self.gate.open.lock().unwrap();
                while !*open {
                    open = self.gate.changed.wait(open).unwrap();
                }
            }
            self.native.open_find_source(path, cancel)
        }
    }
    let temporary = tempfile::tempdir().unwrap();
    let blocked = temporary.path().join("blocked");
    let normal = temporary.path().join("normal");
    fs::create_dir(&blocked).unwrap();
    fs::create_dir(&normal).unwrap();
    let native = Arc::new(LocalFileSystem::new(None));
    let gate = Arc::new(Gate {
        open: Mutex::new(false),
        changed: Condvar::new(),
        entered: AtomicUsize::new(0),
    });
    let release = Release(gate.clone());
    let service = Arc::new(Service {
        native: native.clone(),
        gate: gate.clone(),
    });
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            native,
            events,
        )
        .with_find(service)
        .run(commands),
    );
    for session in 1..=2 {
        send.send(Command::StartFind(Box::new(request(&blocked, session, 1))))
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while gate.entered.load(Ordering::SeqCst) != 2 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    send.send(Command::StartFind(Box::new(request(&normal, 3, 1))))
        .await
        .unwrap();
    send.send(Command::CancelFind {
        session: FindSessionId(3),
        generation: FindGeneration(1),
    })
    .await
    .unwrap();
    loop {
        let event = next_find(&mut receive).await;
        if event.session == FindSessionId(3)
            && let FindUpdate::Finished(summary) = event.update
        {
            assert_eq!(summary.completion, FindCompletion::Cancelled);
            break;
        }
    }
    assert_eq!(gate.entered.load(Ordering::SeqCst), 2);
    send.send(Command::StartFind(Box::new(request(&normal, 4, 1))))
        .await
        .unwrap();
    send.send(Command::StartFind(Box::new(request(&normal, 5, 1))))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::FindRejected(event) = receive.recv().await.unwrap() {
                assert_eq!(event.session, FindSessionId(5));
                assert_eq!(event.error.kind, std::io::ErrorKind::WouldBlock);
                break;
            }
        }
    })
    .await
    .unwrap();
    drop(release);
    drop(send);
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .unwrap()
        .unwrap();
}
