use crate::Engine;
use midnight_core::*;
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::ThreadId,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

const OBSERVE: Duration = Duration::from_secs(8);

struct Files;
impl DirectoryService for Files {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        Ok(())
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        Err(failure(
            path,
            io::ErrorKind::Unsupported,
            "no listing metadata in command transport fixture",
        ))
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}
impl StorageService for Files {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        Err(failure(
            path,
            io::ErrorKind::Unsupported,
            "no volume query in command transport fixture",
        ))
    }
}
fn failure(path: &Path, kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io(
        "admitted sender fixture",
        path,
        io::Error::new(kind, message),
    )
}

#[derive(Debug)]
pub(super) struct Probe {
    callbacks: AtomicUsize,
    callback_threads: Mutex<Vec<ThreadId>>,
    prepared: AtomicBool,
    cancelled: AtomicBool,
    close_entered: AtomicBool,
    closed: AtomicBool,
    release: AtomicBool,
    stop: AtomicBool,
    checked: AtomicBool,
    timed_out: AtomicBool,
    drop_thread: Mutex<Option<ThreadId>>,
}
impl Probe {
    fn new(hold_close: bool) -> Self {
        Self {
            callbacks: AtomicUsize::new(0),
            callback_threads: Mutex::new(Vec::new()),
            prepared: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            close_entered: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            release: AtomicBool::new(!hold_close),
            stop: AtomicBool::new(false),
            checked: AtomicBool::new(false),
            timed_out: AtomicBool::new(false),
            drop_thread: Mutex::new(None),
        }
    }
    fn callback(&self) {
        self.callbacks.fetch_add(1, Ordering::SeqCst);
        self.callback_threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
    }
    pub(super) fn calls(&self) -> usize {
        self.callbacks.load(Ordering::SeqCst)
    }
    pub(super) fn prepared(&self) -> bool {
        self.prepared.load(Ordering::SeqCst)
    }
    pub(super) fn saw_cancel(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
    pub(super) fn close_entered(&self) -> bool {
        self.close_entered.load(Ordering::SeqCst)
    }
    pub(super) fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
    pub(super) fn checked_close(&self) -> bool {
        self.closed() && self.checked.load(Ordering::SeqCst)
    }
    pub(super) fn timed_out(&self) -> bool {
        self.timed_out.load(Ordering::SeqCst)
    }
    pub(super) fn drop_thread(&self) -> Option<ThreadId> {
        *self.drop_thread.lock().unwrap()
    }
    pub(super) fn all_callbacks_off(&self, caller: ThreadId) -> bool {
        self.callback_threads
            .lock()
            .unwrap()
            .iter()
            .all(|thread| *thread != caller)
    }
    pub(super) fn release(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.release.store(true, Ordering::SeqCst);
    }
}
pub(super) struct Release(pub(super) Arc<Probe>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[derive(Debug)]
struct Capture {
    file: Option<File>,
    probe: Arc<Probe>,
    wait_cancel: bool,
    checked_last: bool,
}
impl CapturedTransferSources for Capture {
    fn prepares_audited_native_children(&self) -> bool {
        self.probe.callback();
        true
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        if let Some(mut owner) = Arc::into_inner(self) {
            owner.checked_last = true;
            drop(owner);
        }
        Ok(())
    }
    fn intent(&self) -> TransferIntent {
        self.probe.callback();
        TransferIntent::Copy
    }
    fn root_count(&self) -> usize {
        self.probe.callback();
        1
    }
    fn metadata_available(&self) -> bool {
        self.probe.callback();
        true
    }
    fn prepare(
        &self,
        _: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        self.probe.callback();
        self.probe.prepared.store(true, Ordering::SeqCst);
        if self.wait_cancel {
            let stopped = wait_sync(Duration::from_secs(30), || {
                cancel.is_cancelled() || self.probe.stop.load(Ordering::SeqCst)
            });
            if !stopped {
                self.probe.timed_out.store(true, Ordering::SeqCst);
            }
        }
        if cancel.is_cancelled() {
            self.probe.cancelled.store(true, Ordering::SeqCst);
            Err(failure(
                Path::new(""),
                io::ErrorKind::Interrupted,
                "fixture observed actual Copy cancellation",
            ))
        } else {
            Err(failure(
                Path::new(""),
                io::ErrorKind::Unsupported,
                "fixture has no prepared children",
            ))
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let Some(file) = self.file.take() else {
            return;
        };
        *self.probe.drop_thread.lock().unwrap() = Some(std::thread::current().id());
        self.probe.close_entered.store(true, Ordering::SeqCst);
        if !wait_sync(Duration::from_secs(30), || {
            self.probe.release.load(Ordering::SeqCst)
        }) {
            self.probe.timed_out.store(true, Ordering::SeqCst);
        }
        drop(file);
        self.probe
            .checked
            .store(self.checked_last, Ordering::SeqCst);
        self.probe.closed.store(true, Ordering::SeqCst);
    }
}

pub(super) struct Case {
    _temp: tempfile::TempDir,
    source: PathBuf,
    destination: PathBuf,
    events: Mutex<Option<mpsc::Sender<Event>>>,
}
impl Case {
    pub(super) fn new() -> Self {
        let root = std::env::temp_dir().join("midnight-command-admission/_build/tmp");
        fs::create_dir_all(&root).unwrap();
        let temp = tempfile::Builder::new()
            .prefix("sender-")
            .tempdir_in(root)
            .unwrap();
        let source = temp.path().join("captured-native-file");
        fs::write(&source, b"real captured native ownership").unwrap();
        let destination = temp.path().join("destination");
        fs::create_dir(&destination).unwrap();
        Self {
            _temp: temp,
            source,
            destination,
            events: Mutex::new(None),
        }
    }
    pub(super) fn engine(&self, capacity: usize, native: bool) -> (Engine, mpsc::Receiver<Event>) {
        let (events, receiver) = mpsc::channel(capacity);
        *self.events.lock().unwrap() = Some(events.clone());
        let mut engine = Engine::new(Engine::lifetime().unwrap(), Arc::new(Files), events);
        if native {
            engine
                .configure_audited_transfers()
                .unwrap()
                .install(Arc::new(midnight_platform::LocalFileSystem::new(None)));
        }
        (engine, receiver)
    }
    pub(super) fn invalid_copy(&self, id: u64) -> Command {
        Command::EnqueueCopy(CopyRequest {
            id: OperationId(id),
            start_policy: TransferStartPolicy::Immediate,
            verification: TransferVerification::default(),
            clipboard: FileClipboard {
                paths: vec![self.source.clone()],
                intent: FileClipboardIntent::Copy,
            },
            destination: PathBuf::from("relative-invalid-destination"),
        })
    }
    pub(super) fn valid_copy_waiting(&self, id: u64) -> Command {
        Command::EnqueueCopy(CopyRequest {
            id: OperationId(id),
            start_policy: TransferStartPolicy::WaitForEarlier,
            verification: TransferVerification::default(),
            clipboard: FileClipboard {
                paths: vec![self.source.clone()],
                intent: FileClipboardIntent::Copy,
            },
            destination: self.destination.clone(),
        })
    }
    pub(super) fn copied_bytes_match(&self) -> bool {
        fs::read(self.destination.join(self.source.file_name().unwrap()))
            .ok()
            .is_some_and(|bytes| bytes == b"real captured native ownership")
    }
    pub(super) fn capture(
        &self,
        id: u64,
        wait_cancel: bool,
        hold_close: bool,
    ) -> (Command, Arc<Probe>) {
        let probe = Arc::new(Probe::new(hold_close));
        let command = Command::EnqueueCapturedTransfer(CapturedTransferRequest {
            id: OperationId(id),
            start_policy: TransferStartPolicy::Immediate,
            verification: TransferVerification::default(),
            metadata: TransferMetadataPolicy::ContentOnly,
            destination: self.destination.clone(),
            sources: Arc::new(Capture {
                file: Some(File::open(&self.source).unwrap()),
                probe: probe.clone(),
                wait_cancel,
                checked_last: false,
            }),
        });
        (command, probe)
    }
    pub(super) fn fill_events(&self) {
        self.events
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .try_send(Event::PreferencesLoaded(Ok(Default::default())))
            .unwrap();
    }
    pub(super) fn close_event_sender(&self) {
        self.events.lock().unwrap().take();
    }
}

/// Refusal leaves the original command with the caller. Tests explicitly move
/// that owner to one awaited worker; they never discard an error on the UI task.
pub(super) async fn retire_refused(command: Command) -> Option<u64> {
    tokio::task::spawn_blocking(move || consume_refused(command))
        .await
        .unwrap()
}
pub(super) fn retire_refused_sync(command: Command) -> Option<u64> {
    std::thread::spawn(move || consume_refused(command))
        .join()
        .unwrap()
}
fn consume_refused(command: Command) -> Option<u64> {
    match command {
        Command::EnqueueCapturedTransfer(request) => {
            let id = request.id.0;
            assert_eq!(request.start_policy, TransferStartPolicy::Immediate);
            assert_eq!(request.metadata, TransferMetadataPolicy::ContentOnly);
            assert!(request.destination.is_absolute());
            request.sources.release_on_worker().unwrap();
            Some(id)
        }
        command => {
            drop(command);
            None
        }
    }
}
pub(super) fn wait_sync(timeout: Duration, condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while !condition() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    true
}
pub(super) async fn wait_async(timeout: Duration, condition: impl Fn() -> bool) -> bool {
    tokio::time::timeout(timeout, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .is_ok()
}
pub(super) async fn terminal(
    events: &mut mpsc::Receiver<Event>,
    id: u64,
) -> Option<Arc<CopyProgress>> {
    tokio::time::timeout(OBSERVE, async {
        while let Some(event) = events.recv().await {
            if let Event::Copy(CopyEvent::Progress(progress)) = event
                && progress.id == OperationId(id)
                && progress.state.is_terminal()
            {
                return Some(progress);
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}
