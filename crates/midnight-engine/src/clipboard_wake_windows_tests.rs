//! Idle wakeups of the native clipboard worker with the real Windows session (review
//! finding E6-4). The platform-neutral worker tests are in `clipboard_wake_tests.rs`.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Counts worker loop iterations (one `service_pending` per iteration with an open
/// session) and native waits of the real platform session.
struct Counted {
    inner: Box<dyn ClipboardSession>,
    iterations: Arc<AtomicUsize>,
    waits: Arc<AtomicUsize>,
    text: Arc<Mutex<Option<String>>>,
}
impl ClipboardSession for Counted {
    fn copy_text(&mut self, text: &str) -> Result<(), FsError> {
        self.inner.copy_text(text)
    }
    fn read_text(&mut self) -> Result<String, FsError> {
        let text = self.inner.read_text()?;
        *self.text.lock().unwrap() = Some(text.clone());
        Ok(text)
    }
    fn copy_files(&mut self, files: &FileClipboard) -> Result<(), FsError> {
        self.inner.copy_files(files)
    }
    fn read_files(&mut self) -> Result<FileClipboard, FsError> {
        self.inner.read_files()
    }
    fn service_pending(&mut self) -> Result<(), FsError> {
        self.iterations.fetch_add(1, Ordering::Relaxed);
        self.inner.service_pending()
    }
    fn owns_clipboard(&self) -> bool {
        self.inner.owns_clipboard()
    }
    fn wait_pending(&mut self, timeout: Duration) -> Result<(), FsError> {
        self.waits.fetch_add(1, Ordering::Relaxed);
        self.inner.wait_pending(timeout)
    }
    fn waker(&self) -> Option<ClipboardWaker> {
        self.inner.waker()
    }
}
struct CountedService {
    inner: Arc<dyn ClipboardService>,
    iterations: Arc<AtomicUsize>,
    waits: Arc<AtomicUsize>,
    text: Arc<Mutex<Option<String>>>,
}
impl ClipboardService for CountedService {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        Ok(Box::new(Counted {
            inner: self.inner.open()?,
            iterations: self.iterations.clone(),
            waits: self.waits.clone(),
            text: self.text.clone(),
        }))
    }
}

/// The event's variant and outcome for the measurement output. Never its payload: a text
/// read carries the user's clipboard text, and file events carry paths.
fn outcome(event: Option<&Event>) -> String {
    let (variant, failure) = match event {
        Some(Event::PathClipboardTextRead { result, .. }) => {
            ("PathClipboardTextRead", result.as_ref().err())
        }
        Some(Event::ClipboardFilesRead { result, .. }) => {
            ("ClipboardFilesRead", result.as_ref().err())
        }
        Some(Event::DesktopActionFinished { result, .. }) => {
            ("DesktopActionFinished", result.as_ref().err())
        }
        Some(_) => return "an unexpected event".into(),
        None => return "no event (the worker stopped)".into(),
    };
    match failure {
        None => format!("{variant} Ok"),
        Some(failure) => format!("{variant} Err({:?})", failure.kind),
    }
}

/// Before/after measurement for review finding E6-4: worker loop iterations and native
/// waits over two idle seconds after a read (no ownership taken) and after a file copy
/// (owned clipboard). It prints counts and event outcomes only, never clipboard contents.
/// Run serially: `cargo test -p midnight-engine --lib -- --ignored --nocapture
/// clipboard::wake_windows_tests::measure_idle_wakeups_of_the_real_windows_session`.
#[test]
#[ignore = "manual measurement: opens the real Windows clipboard and temporarily replaces its contents"]
fn measure_idle_wakeups_of_the_real_windows_session() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let iterations = Arc::new(AtomicUsize::new(0));
    let waits = Arc::new(AtomicUsize::new(0));
    let text = Arc::new(Mutex::new(None));
    let (sender, mut events) = mpsc::channel(4);
    let mut controller = Controller::testing(
        Some(Arc::new(CountedService {
            inner: Arc::new(midnight_platform::NativeClipboard),
            iterations: iterations.clone(),
            waits: waits.clone(),
            text: text.clone(),
        })),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
    );
    let mut tasks = JoinSet::new();
    let window = Duration::from_secs(2);
    let mut idle = |label: &str| {
        let event = runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(5), events.recv()).await })
            .expect("clipboard event");
        let (loops, waited) = (
            iterations.load(Ordering::Relaxed),
            waits.load(Ordering::Relaxed),
        );
        // A measurement window, not synchronization: count what the idle worker does.
        std::thread::sleep(window);
        eprintln!(
            "{label}: {} worker loop iterations, {} native waits in {window:?} ({})",
            iterations.load(Ordering::Relaxed) - loops,
            waits.load(Ordering::Relaxed) - waited,
            outcome(event.as_ref()),
        );
    };
    runtime.block_on(controller.read_text(DesktopRequestId(1), &mut tasks));
    idle("after a read (no ownership)");
    let path = std::env::temp_dir().join("midnight idle wakeup 文档.txt");
    let view = Arc::new(midnight_core::DirectoryIndex::from_entries(&[
        midnight_core::Entry::new(
            midnight_core::EntryId(1),
            path.clone(),
            midnight_core::EntryKind::File,
        ),
    ]));
    let files = SelectedFiles {
        selection: Arc::default(),
        view,
        fallback: Some(path),
    };
    runtime.block_on(controller.copy_files(DesktopRequestId(2), files, &mut tasks));
    idle("after a file copy (owned clipboard)");
    // Leave the user's previous Unicode text on the clipboard again.
    if let Some(previous) = text.lock().unwrap().take() {
        runtime.block_on(controller.copy(DesktopRequestId(3), previous, &mut tasks));
        let restored = runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(5), events.recv()).await })
            .expect("restored clipboard text");
        eprintln!("restored the previous text: {}", outcome(restored.as_ref()));
    }
    controller.shutdown();
    runtime.block_on(async { while tasks.join_next().await.is_some() {} });
}
