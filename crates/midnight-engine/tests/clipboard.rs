use midnight_core::*;
use midnight_engine::Engine;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

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
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(8);
    let engine = tokio::spawn(
        Engine::new(
            Arc::new(midnight_platform::LocalFileSystem::new(None)),
            sender,
        )
        .with_clipboard(clipboard.clone())
        .run(receiver),
    );
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
    let (commands, receiver) = mpsc::channel(4);
    let (sender, mut events) = mpsc::channel(4);
    let engine = tokio::spawn(
        Engine::new(
            Arc::new(midnight_platform::LocalFileSystem::new(None)),
            sender,
        )
        .run(receiver),
    );
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
    let (commands, receiver) = mpsc::channel(4);
    let (sender, mut events) = mpsc::channel(4);
    let engine = tokio::spawn(
        Engine::new(
            Arc::new(midnight_platform::LocalFileSystem::new(None)),
            sender,
        )
        .with_clipboard(clipboard.clone())
        .run(receiver),
    );
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
