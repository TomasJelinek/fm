use midnight_core::*;
use midnight_engine::Engine;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

async fn completion(
    events: &mut mpsc::Receiver<Event>,
) -> (DesktopRequestId, Result<DesktopActionResult, FsError>) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Event::DesktopActionFinished { request, result } =
                events.recv().await.expect("engine event stream closed")
            {
                return (request, result);
            }
        }
    })
    .await
    .expect("clipboard request did not complete")
}

#[derive(Default)]
struct Clipboard {
    calls: Arc<Mutex<Vec<(std::thread::ThreadId, String)>>>,
}

struct Session(Arc<Mutex<Vec<(std::thread::ThreadId, String)>>>);

impl ClipboardSession for Session {
    fn copy_text(&mut self, text: &str) -> Result<(), FsError> {
        self.0
            .lock()
            .unwrap()
            .push((std::thread::current().id(), text.into()));
        Ok(())
    }
}

impl ClipboardService for Clipboard {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        Ok(Box::new(Session(self.calls.clone())))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_copy_preserves_literal_unicode_and_serializes_on_backend_worker() {
    let clipboard = Arc::new(Clipboard::default());
    let caller = std::thread::current().id();

    let (sender, mut events) = mpsc::channel(8);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
        sender,
    );
    let (commands, receiver) = engine.command_channel(8).unwrap();
    engine
        .configure_clipboard()
        .unwrap()
        .install(clipboard.clone());
    let engine = tokio::spawn(engine.run(receiver));
    let expected = ["文档 e\u{301} 👩🏽‍💻", "1000000000000", "-rwsr-xr-t (4755)"];
    for (index, text) in expected.iter().enumerate() {
        commands
            .send(Command::CopyText {
                request: DesktopRequestId(index as u64 + 1),
                text: (*text).into(),
            })
            .await
            .unwrap();
    }
    for index in 0..expected.len() {
        let (request, result) = completion(&mut events).await;
        assert_eq!(request, DesktopRequestId(index as u64 + 1));
        assert!(matches!(result, Ok(DesktopActionResult::ClipboardCopied)));
    }
    let calls = clipboard.calls.lock().unwrap().clone();
    assert_eq!(
        calls
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        calls
            .iter()
            .all(|(worker, _)| *worker != caller && *worker == calls[0].0)
    );
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_clipboard_service_reports_failure_without_claiming_a_copy() {
    let (sender, mut events) = mpsc::channel(4);
    let mut command_engine_1 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
        sender,
    );
    let (commands, receiver) = command_engine_1.command_channel(4).unwrap();
    let engine = tokio::spawn(command_engine_1.run(receiver));
    commands
        .send(Command::CopyText {
            request: DesktopRequestId(7),
            text: "literal".into(),
        })
        .await
        .unwrap();
    let (request, result) = completion(&mut events).await;
    assert_eq!(request, DesktopRequestId(7));
    assert!(result.is_err());
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_clipboard_text_never_reaches_the_native_service() {
    let clipboard = Arc::new(Clipboard::default());

    let (sender, mut events) = mpsc::channel(4);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
        sender,
    );
    let (commands, receiver) = engine.command_channel(4).unwrap();
    engine
        .configure_clipboard()
        .unwrap()
        .install(clipboard.clone());
    let engine = tokio::spawn(engine.run(receiver));
    for text in ["before\0after".into(), "x".repeat(4 * 1024 * 1024 + 1)] {
        commands
            .send(Command::CopyText {
                request: DesktopRequestId(1),
                text,
            })
            .await
            .unwrap();
        let (request, result) = completion(&mut events).await;
        assert_eq!(request, DesktopRequestId(1));
        assert!(matches!(result, Err(error) if error.kind == std::io::ErrorKind::InvalidInput));
    }
    assert!(clipboard.calls.lock().unwrap().is_empty());
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[derive(Default)]
struct ClipboardLifetime {
    copied: tokio::sync::Notify,
    dropped: tokio::sync::Notify,
}

struct ObservedClipboard(Arc<ClipboardLifetime>);
struct ObservedSession(Arc<ClipboardLifetime>);
impl ClipboardSession for ObservedSession {
    fn copy_text(&mut self, text: &str) -> Result<(), FsError> {
        assert_eq!(text, "captured clipboard text");
        self.0.copied.notify_one();
        Ok(())
    }
}
impl Drop for ObservedSession {
    fn drop(&mut self) {
        self.0.dropped.notify_one();
    }
}
impl ClipboardService for ObservedClipboard {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        Ok(Box::new(ObservedSession(self.0.clone())))
    }
}

/// A session whose close (its drop on the worker thread) waits for the test, as a real
/// close can: on Linux it hands copied text over to a clipboard manager, or finishes an
/// X11 transfer, before the process may exit.
struct ClosingClipboard(Arc<Closing>);
struct ClosingSession(Arc<Closing>);
struct Closing {
    started: Mutex<Option<oneshot::Sender<()>>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    closed: AtomicBool,
}
impl ClipboardSession for ClosingSession {
    fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
        Ok(())
    }
}
impl Drop for ClosingSession {
    fn drop(&mut self) {
        if let Some(started) = self.0.started.lock().unwrap().take() {
            let _ = started.send(());
        }
        // Bounded, so a failing test cannot strand the worker thread.
        let _ = self
            .0
            .release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10));
        self.0.closed.store(true, Ordering::SeqCst);
    }
}
impl ClipboardService for ClosingClipboard {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        Ok(Box::new(ClosingSession(self.0.clone())))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn normal_shutdown_returns_only_after_the_clipboard_session_closed() {
    let (started, closing) = oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let session = Arc::new(Closing {
        started: Mutex::new(Some(started)),
        release: Mutex::new(released),
        closed: AtomicBool::new(false),
    });
    let (sender, mut events) = mpsc::channel(8);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
        sender,
    );
    let (commands, receiver) = engine.command_channel(8).unwrap();
    engine
        .configure_clipboard()
        .unwrap()
        .install(Arc::new(ClosingClipboard(session.clone())));
    let mut engine = tokio::spawn(engine.run(receiver));
    commands
        .send(Command::CopyText {
            request: DesktopRequestId(1),
            text: "kept after exit".into(),
        })
        .await
        .unwrap();
    let (request, result) = completion(&mut events).await;
    assert_eq!(request, DesktopRequestId(1));
    assert!(matches!(result, Ok(DesktopActionResult::ClipboardCopied)));
    // The application stops consuming events before it sends Shutdown.
    drop(events);
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), closing)
        .await
        .expect("shutdown did not close the clipboard session")
        .unwrap();
    // The worker thread is still closing its session. The run waits for it (at most
    // the engine's five-second actor bound) instead of aborting only its waiter.
    let early = tokio::time::timeout(Duration::from_millis(250), &mut engine).await;
    release.send(()).unwrap();
    assert!(
        early.is_err(),
        "Engine::run returned while the clipboard session was still closing"
    );
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .expect("the engine did not stop")
        .unwrap();
    assert!(
        session.closed.load(Ordering::SeqCst),
        "Engine::run returned before the clipboard session closed"
    );
}

#[tokio::test]
async fn aborting_engine_releases_clipboard_session_while_output_remains_full() {
    let clipboard = Arc::new(ClipboardLifetime::default());

    let (sender, events) = mpsc::channel(1);
    sender.try_send(Event::VolumesUpdated(Vec::new())).unwrap();
    let mut backend = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
        sender,
    );
    let (commands, receiver) = backend.command_channel(2).unwrap();
    backend
        .configure_clipboard()
        .unwrap()
        .install(Arc::new(ObservedClipboard(clipboard.clone())));
    let engine = tokio::spawn(backend.run(receiver));
    commands
        .send(Command::CopyText {
            request: DesktopRequestId(1),
            text: "captured clipboard text".into(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), clipboard.copied.notified())
        .await
        .unwrap();
    engine.abort();
    assert!(engine.await.unwrap_err().is_cancelled());
    let released = tokio::time::timeout(Duration::from_secs(3), clipboard.dropped.notified())
        .await
        .is_ok();
    // A failing assertion must not strand the old blocking sender at runtime teardown.
    drop(events);
    if !released {
        tokio::time::timeout(Duration::from_secs(3), clipboard.dropped.notified())
            .await
            .unwrap();
    }
    assert!(
        released,
        "Clipboard session outlived its aborted engine behind full output"
    );
}
