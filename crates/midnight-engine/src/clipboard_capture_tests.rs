//! Opaque sources stay on backend workers through native publication and refusal.
use super::*;
use midnight_core::CapturedClipboardFiles;
use std::sync::Mutex;

#[derive(Debug)]
struct Tracked {
    files: FileClipboard,
    intent: FileClipboardIntent,
    invalid: Cancellation,
    dropped: Arc<Mutex<Option<std::thread::ThreadId>>>,
}
impl CapturedClipboardFiles for Tracked {
    fn intent(&self) -> FileClipboardIntent {
        self.intent
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<FileClipboard, FsError> {
        if cancel.is_cancelled() || self.invalid.is_cancelled() {
            return Err(error(
                io::ErrorKind::Interrupted,
                "Source context was cancelled.",
            ));
        }
        Ok(self.files.clone())
    }
}
impl Drop for Tracked {
    fn drop(&mut self) {
        *self.dropped.lock().unwrap() = Some(std::thread::current().id());
    }
}

struct Native {
    live: std::sync::Weak<Tracked>,
    cancel_on_open: Option<Cancellation>,
    copies: Arc<Mutex<Vec<FileClipboard>>>,
}
impl ClipboardService for Native {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        if let Some(cancel) = &self.cancel_on_open {
            cancel.cancel();
        }
        Ok(Box::new(Self {
            live: self.live.clone(),
            cancel_on_open: None,
            copies: self.copies.clone(),
        }))
    }
}
impl ClipboardSession for Native {
    fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
        panic!("file action")
    }
    fn copy_files(&mut self, files: &FileClipboard) -> Result<(), FsError> {
        assert!(
            self.live.upgrade().is_some(),
            "source authority must cover native write"
        );
        self.copies.lock().unwrap().push(files.clone());
        Ok(())
    }
}

fn source(intent: FileClipboardIntent) -> Arc<Tracked> {
    Arc::new(Tracked {
        files: FileClipboard {
            paths: vec![std::env::temp_dir().join("資料 🦀 $(literal) '.txt")],
            intent,
        },
        intent,
        invalid: Cancellation::default(),
        dropped: Arc::default(),
    })
}

async fn publish(
    source: Arc<Tracked>,
    cancel: Cancellation,
    on_open: Option<Cancellation>,
) -> (Event, Vec<FileClipboard>, std::thread::ThreadId) {
    let copies = Arc::new(Mutex::new(Vec::new()));
    let dropped = source.dropped.clone();
    let (sender, mut events) = mpsc::channel(1);
    let mut controller = Controller::testing(
        Some(Arc::new(Native {
            live: Arc::downgrade(&source),
            cancel_on_open: on_open,
            copies: copies.clone(),
        })),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
    );
    let mut tasks = JoinSet::new();
    controller
        .copy_captured(DesktopRequestId(31), source, cancel, &mut tasks)
        .await;
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    controller.shutdown();
    while tasks.join_next().await.is_some() {}
    let copied = copies.lock().unwrap().clone();
    let retired = dropped.lock().unwrap().expect("source retired");
    (event, copied, retired)
}

#[tokio::test(flavor = "current_thread")]
async fn captured_clipboard_publishes_literal_paths_and_retains_source_through_native_write() {
    for intent in [FileClipboardIntent::Copy, FileClipboardIntent::Move] {
        let source = source(intent);
        let expected = source.files.clone();
        let (event, copied, retired) = publish(source, Cancellation::default(), None).await;
        assert!(matches!(
            event,
            Event::DesktopActionFinished {
                result: Ok(DesktopActionResult::ClipboardCopied),
                ..
            }
        ));
        assert_eq!(copied, [expected]);
        assert_ne!(retired, std::thread::current().id());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn captured_clipboard_revalidates_request_and_provider_after_native_open() {
    for cancel_source in [false, true] {
        let source = source(FileClipboardIntent::Copy);
        let request_cancel = Cancellation::default();
        let invalidated = if cancel_source {
            source.invalid.clone()
        } else {
            request_cancel.clone()
        };
        let (event, copied, retired) = publish(source, request_cancel, Some(invalidated)).await;
        assert!(
            matches!(event, Event::DesktopActionFinished { result: Err(error), .. } if error.kind == io::ErrorKind::Interrupted)
        );
        assert!(copied.is_empty());
        assert_ne!(retired, std::thread::current().id());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn captured_clipboard_rejects_changed_intent_before_native_write() {
    let mut source = source(FileClipboardIntent::Copy);
    Arc::get_mut(&mut source).unwrap().files.intent = FileClipboardIntent::Move;
    let (event, copied, _) = publish(source, Cancellation::default(), None).await;
    assert!(
        matches!(event, Event::DesktopActionFinished { result: Err(error), .. } if error.kind == io::ErrorKind::InvalidData)
    );
    assert!(copied.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn clipboard_queue_refusal_retires_provider_and_generated_sources_off_dispatcher() {
    for generated in [false, true] {
        let (native, _queued) = mpsc::channel(1);
        assert!(
            native
                .try_send(Request {
                    id: DesktopRequestId(1),
                    action: Action::Text("full".into())
                })
                .is_ok()
        );
        let (sender, mut events) = mpsc::channel(1);
        let mut controller = Controller::testing(
            None,
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
        );
        controller.sender = Some(native);
        let source = source(FileClipboardIntent::Copy);
        let observed = source.dropped.clone();
        let mut tasks = JoinSet::new();
        if generated {
            controller
                .copy_generated(
                    DesktopRequestId(32),
                    source,
                    Cancellation::default(),
                    &mut tasks,
                )
                .await;
        } else {
            controller
                .copy_captured(
                    DesktopRequestId(32),
                    source,
                    Cancellation::default(),
                    &mut tasks,
                )
                .await;
        }
        let event = events.recv().await.unwrap();
        assert!(
            matches!(event, Event::DesktopActionFinished { result: Err(error), .. } if error.kind == io::ErrorKind::WouldBlock)
        );
        assert_ne!(
            observed.lock().unwrap().expect("refused source retired"),
            std::thread::current().id()
        );
        assert!(
            tasks.is_empty(),
            "refusal must not leave an unbounded retirement job"
        );
        controller.shutdown();
    }
}

impl midnight_core::GeneratedTextSource for Tracked {
    fn length(&self) -> u64 {
        panic!("refused source must not be inspected")
    }
    fn read_at(&self, _: u64, _: &mut [u8]) -> io::Result<usize> {
        panic!("refused source must not be read")
    }
}

#[tokio::test(flavor = "current_thread")]
async fn engine_shutdown_retires_unprocessed_clipboard_sources_off_dispatcher() {
    let (sender, mut receiver) = mpsc::channel(2);
    let source = source(FileClipboardIntent::Copy);
    let observed = source.dropped.clone();
    sender
        .try_send(midnight_core::Command::CopyCapturedFiles {
            request: DesktopRequestId(33),
            source,
            cancel: Cancellation::default(),
        })
        .unwrap();
    receiver.close();
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    crate::retire_commands(
        domain
            .try_reserve()
            .unwrap()
            .own(crate::engine_commands::CommandInbox(receiver)),
    )
    .await;
    assert!(sender.is_closed());
    assert_ne!(
        observed.lock().unwrap().expect("queued source retired"),
        std::thread::current().id()
    );
}
