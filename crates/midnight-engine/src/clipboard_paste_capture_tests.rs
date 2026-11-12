//! Captured Paste resolves its destination on the native clipboard worker.
use super::*;
use midnight_core::CapturedClipboardFiles;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
struct Destination {
    files: FileClipboard,
    revoked: Cancellation,
    calls: AtomicUsize,
    dropped: Arc<Mutex<Option<std::thread::ThreadId>>>,
}
impl CapturedClipboardFiles for Destination {
    fn intent(&self) -> FileClipboardIntent {
        FileClipboardIntent::Copy
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<FileClipboard, FsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if cancel.is_cancelled() || self.revoked.is_cancelled() {
            return Err(error(io::ErrorKind::Interrupted, "Destination revoked."));
        }
        Ok(self.files.clone())
    }
}
impl Drop for Destination {
    fn drop(&mut self) {
        *self.dropped.lock().unwrap() = Some(std::thread::current().id());
    }
}
struct Native {
    destination: std::sync::Weak<Destination>,
    on_open: Option<Cancellation>,
    on_read: Option<Cancellation>,
    reads: Arc<AtomicUsize>,
}
impl ClipboardService for Native {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        if let Some(cancel) = &self.on_open {
            cancel.cancel();
        }
        Ok(Box::new(Self {
            destination: self.destination.clone(),
            on_open: None,
            on_read: self.on_read.clone(),
            reads: self.reads.clone(),
        }))
    }
}
impl ClipboardSession for Native {
    fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
        panic!("Paste must only read")
    }
    fn read_files(&mut self) -> Result<FileClipboard, FsError> {
        assert!(
            self.destination.upgrade().is_some(),
            "native read must retain destination authority"
        );
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(cancel) = &self.on_read {
            cancel.cancel();
        }
        Ok(FileClipboard {
            paths: vec![std::env::temp_dir().join("copied 文档.txt")],
            intent: FileClipboardIntent::Copy,
        })
    }
}

async fn read(
    count: usize,
    invalid_intent: bool,
    cancel_on_open: bool,
    revoke_on_read: bool,
) -> (Event, usize, std::thread::ThreadId) {
    let request_cancel = Cancellation::default();
    let revoked = Cancellation::default();
    let dropped = Arc::new(Mutex::new(None));
    let destination = Arc::new(Destination {
        files: FileClipboard {
            paths: (0..count)
                .map(|index| std::env::temp_dir().join(format!("captured folder 文档 {index}")))
                .collect(),
            intent: if invalid_intent {
                FileClipboardIntent::Move
            } else {
                FileClipboardIntent::Copy
            },
        },
        revoked: revoked.clone(),
        calls: AtomicUsize::new(0),
        dropped: dropped.clone(),
    });
    let reads = Arc::new(AtomicUsize::new(0));
    let service = Native {
        destination: Arc::downgrade(&destination),
        on_open: cancel_on_open.then(|| request_cancel.clone()),
        on_read: revoke_on_read.then_some(revoked),
        reads: reads.clone(),
    };
    let (sender, mut events) = mpsc::channel(1);
    let mut controller = Controller::testing(
        Some(Arc::new(service)),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
    );
    let mut tasks = JoinSet::new();
    controller
        .read_captured_files(
            DesktopRequestId(91),
            destination,
            request_cancel,
            &mut tasks,
        )
        .await;
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    controller.shutdown();
    while tasks.join_next().await.is_some() {}
    let owner = dropped.lock().unwrap().expect("destination retired");
    (event, reads.load(Ordering::SeqCst), owner)
}

#[tokio::test(flavor = "current_thread")]
async fn captured_paste_uses_literal_destination_and_keeps_ownership_off_the_caller() {
    let (event, reads, owner) = read(1, false, false, false).await;
    assert_eq!(reads, 1);
    assert_ne!(owner, std::thread::current().id());
    assert!(
        matches!(event, Event::ClipboardFilesRead { destination, result: Ok(_), .. }
        if destination == std::env::temp_dir().join("captured folder 文档 0"))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn captured_paste_refuses_ambiguous_intent_and_cancelled_destination_before_native_read() {
    for (count, invalid, cancelled) in [(2, false, false), (1, true, false), (1, false, true)] {
        let (event, reads, owner) = read(count, invalid, cancelled, false).await;
        assert_eq!(reads, 0);
        assert_ne!(owner, std::thread::current().id());
        assert!(matches!(
            event,
            Event::ClipboardFilesRead { result: Err(_), .. }
        ));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn captured_paste_revalidates_after_blocking_native_read() {
    let (event, reads, owner) = read(1, false, false, true).await;
    assert_eq!(reads, 1);
    assert_ne!(owner, std::thread::current().id());
    assert!(matches!(
        event,
        Event::ClipboardFilesRead { result: Err(_), .. }
    ));
}
