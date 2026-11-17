use super::*;
use crate::{CommittedSnapshot, SnapshotState};
use std::{
    ffi::OsStr,
    fs::{self, File},
    sync::Condvar,
    time::Instant,
};

#[derive(Debug, Default)]
struct Observed {
    entered: bool,
    released: bool,
    closed: bool,
    read: bool,
    close_before_read: bool,
    checked_last: bool,
    timed_out: bool,
    drop_thread: Option<std::thread::ThreadId>,
}

#[derive(Debug, Default)]
pub(super) struct Trace {
    state: Mutex<Observed>,
    changed: Condvar,
}
impl Trace {
    fn close_file(&self, file: File, checked_last: bool) {
        // Keep the actual native File alive through the hold, then record only
        // after its synchronous destructor returns.
        let mut state = self.state.lock().unwrap();
        state.entered = true;
        state.checked_last = checked_last;
        state.drop_thread = Some(std::thread::current().id());
        self.changed.notify_all();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !state.released {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                state.timed_out = true;
                break;
            }
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
        drop(state);
        drop(file);
        self.state.lock().unwrap().closed = true;
        self.changed.notify_all();
    }
    fn native_read(&self) {
        let mut state = self.state.lock().unwrap();
        if !state.read {
            state.close_before_read = state.closed;
        }
        state.read = true;
        self.changed.notify_all();
    }
    pub(super) fn release(&self) {
        self.state.lock().unwrap().released = true;
        self.changed.notify_all();
    }
    pub(super) fn closed(&self) -> bool {
        self.state.lock().unwrap().closed
    }
    pub(super) fn checked_last_release(&self) -> bool {
        self.state.lock().unwrap().checked_last
    }
    pub(super) fn close_precedes_read(&self) -> bool {
        self.state.lock().unwrap().close_before_read
    }
    pub(super) fn timed_out(&self) -> bool {
        self.state.lock().unwrap().timed_out
    }
    pub(super) fn drop_thread(&self) -> Option<std::thread::ThreadId> {
        self.state.lock().unwrap().drop_thread
    }
    pub(super) fn wait_entered_sync(&self, timeout: Duration) -> bool {
        wait_sync(timeout, || self.state.lock().unwrap().entered)
    }
    pub(super) fn wait_closed_sync(&self, timeout: Duration) -> bool {
        wait_sync(timeout, || self.closed())
    }
    pub(super) async fn wait_entered(&self, timeout: Duration) -> bool {
        wait_async(timeout, || self.state.lock().unwrap().entered).await
    }
    pub(super) async fn wait_closed(&self, timeout: Duration) -> bool {
        wait_async(timeout, || self.closed()).await
    }
    pub(super) async fn wait_read(&self, timeout: Duration) -> bool {
        wait_async(timeout, || self.state.lock().unwrap().read).await
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
async fn wait_async(timeout: Duration, condition: impl Fn() -> bool) -> bool {
    tokio::time::timeout(timeout, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .is_ok()
}
pub(super) struct Release(pub(super) Arc<Trace>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[derive(Debug)]
struct CapturedFile {
    files: FileClipboard,
    file: Option<File>,
    checked_last: bool,
    trace: Arc<Trace>,
}
impl CapturedClipboardFiles for CapturedFile {
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        if let Some(mut owner) = Arc::into_inner(self) {
            owner.checked_last = true;
            drop(owner);
        }
        Ok(())
    }
    fn intent(&self) -> FileClipboardIntent {
        FileClipboardIntent::Copy
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<FileClipboard, FsError> {
        if cancel.is_cancelled() {
            return Err(error(io::ErrorKind::Interrupted, "capture cancelled"));
        }
        Ok(self.files.clone())
    }
}
impl Drop for CapturedFile {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            self.trace.close_file(file, self.checked_last);
        }
    }
}

pub(super) struct Case {
    _temp: tempfile::TempDir,
    source: PathBuf,
    b: PathBuf,
    pub(super) destination: PathBuf,
    pub(super) bytes: Vec<u8>,
    pub(super) trace: Arc<Trace>,
}
impl Case {
    pub(super) fn new() -> Self {
        let root = std::env::temp_dir().join("midnight-accepted-drop/_build/tmp");
        fs::create_dir_all(&root).unwrap();
        let temp = tempfile::Builder::new()
            .prefix("retirement-")
            .tempdir_in(root)
            .unwrap();
        let source = temp.path().join("accepted-a");
        let b = temp.path().join("successor-b");
        let destination = temp.path().join("destination");
        fs::create_dir(&destination).unwrap();
        fs::write(&source, b"captured native file A").unwrap();
        let bytes = vec![0x62; 128 * 1024 + 7];
        fs::write(&b, &bytes).unwrap();
        Self {
            _temp: temp,
            source,
            b,
            destination,
            bytes,
            trace: Arc::default(),
        }
    }
    fn capture(&self) -> Arc<dyn CapturedClipboardFiles> {
        Arc::new(CapturedFile {
            files: FileClipboard {
                paths: vec![self.source.clone()],
                intent: FileClipboardIntent::Copy,
            },
            file: Some(File::open(&self.source).unwrap()),
            checked_last: false,
            trace: self.trace.clone(),
        })
    }
    pub(super) fn request(&self, id: u64) -> CopyRequest {
        CopyRequest {
            id: OperationId(id),
            start_policy: TransferStartPolicy::WaitForEarlier,
            verification: TransferVerification::default(),
            clipboard: FileClipboard {
                paths: vec![self.b.clone()],
                intent: FileClipboardIntent::Copy,
            },
            destination: self.destination.clone(),
        }
    }
    pub(super) fn output(&self) -> PathBuf {
        self.destination.join("successor-b")
    }
    pub(super) fn service(&self) -> Arc<TaggedService> {
        Arc::new(TaggedService {
            native: midnight_platform::LocalFileSystem::new(None),
            b: self.b.clone(),
            trace: self.trace.clone(),
        })
    }
}

pub(super) struct TaggedService {
    native: midnight_platform::LocalFileSystem,
    b: PathBuf,
    trace: Arc<Trace>,
}
// Only synchronous native owners are delegated; Trace holds metadata, never a
// native owner or a background cleanup tail. CapturedFile has its own release.
impl AuditedSynchronousTransferService for TaggedService {}
impl TaggedService {
    fn wrap(&self, source: Box<dyn TransferSource>) -> Box<dyn TransferSource> {
        Box::new(TaggedSource {
            source,
            b: self.b.clone(),
            trace: self.trace.clone(),
        })
    }
}
impl TransferService for TaggedService {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.native
            .prepare_transfer_source(path, cancel)
            .map(|source| self.wrap(source))
    }
    fn prepare_transfer_source_with_links(
        &self,
        path: &Path,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.native
            .prepare_transfer_source_with_links(path, links, cancel)
            .map(|source| self.wrap(source))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.native.prepare_transfer_destination(path, cancel)
    }
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.native.prepare_checksum_directory(path, cancel)
    }
}
struct TaggedSource {
    source: Box<dyn TransferSource>,
    b: PathBuf,
    trace: Arc<Trace>,
}
impl TransferSource for TaggedSource {
    fn entry(&self) -> &TransferEntry {
        self.source.entry()
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        self.source.native_identity()
    }
    fn native_namespace_scope(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        self.source.native_namespace_scope(cancel)
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.source.revalidate(cancel)
    }
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.source.discover(cancel, visit)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        let count = self.source.read_chunk(buffer, cancel)?;
        if count != 0 && self.source.entry().item.path == self.b {
            assert!(buffer[..count].iter().all(|byte| *byte == 0x62));
            self.trace.native_read();
        }
        Ok(count)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.source.next_child(cancel)
    }
    fn open_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.source.open_child(name, cancel)
    }
    fn supports_read_retry(&self) -> bool {
        self.source.supports_read_retry()
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.source.retry_read_from(offset, cancel)
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        self.source.seek_for_validation(offset, cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.source.rewind_for_validation(cancel)
    }
    fn prepare_modified_time(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferModifiedTimePin>, FsError> {
        self.source.prepare_modified_time(cancel)
    }
}

pub(super) struct DropHarness {
    pub(super) controller: Controller,
    events: mpsc::Receiver<DragDropEvent>,
    tasks: JoinSet<()>,
}
impl DropHarness {
    pub(super) fn new(admission: copy_operations::Admission, case: &Case) -> Self {
        let snapshot = |path: PathBuf, navigation| SnapshotState {
            committed: Some(CommittedSnapshot {
                navigation,
                path,
                entries: Arc::from([]),
                sizes: Arc::default(),
            }),
            ..Default::default()
        };
        let snapshots = crate::test_snapshots([
            snapshot(case.source.parent().unwrap().to_owned(), NavigationId(1)),
            snapshot(case.destination.clone(), NavigationId(2)),
        ]);
        let (outbox, _unused) = mpsc::channel(4);
        let mut tasks = JoinSet::new();
        let mut controller = Controller::new(
            crate::engine_lifetime::DragRetirement::testing(),
            None,
            None,
            snapshots.clone(),
            admission.clone(),
            Outbox {
                sender: outbox,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        // Invoke the same production worker function directly without starting
        // the periodic resolver. This gives explicit old-gesture drain ordering.
        let (events, receiver) = mpsc::channel(8);
        controller.events = events.clone();
        let host = DragHost {
            generation: 1,
            native: NativeDragWindow::Unsupported,
            lifetime: Cancellation::default(),
        };
        let targets = Arc::new(Mutex::new(DropTargets::default()));
        let sink = Arc::new(Sink {
            host: host.clone(),
            targets: targets.clone(),
            snapshots,
            pending: controller.pending.clone(),
            capacity: admission,
            events,
            next: controller.next.clone(),
            hover: controller.hover.clone(),
            cancel: controller.cancel.clone(),
        });
        let captured = case.capture();
        let files = Arc::new(captured.resolve(&Cancellation::default()).unwrap());
        {
            let mut state = controller.state.lock().unwrap();
            state.host = Some(Host {
                host,
                targets,
                sink,
            });
            state.source = Some(Source {
                id: DragId(1),
                pane: PaneId::Left,
                navigation: NavigationId(1),
                cancel: Cancellation::default(),
                files: Some(files),
                captured: Some(captured),
                running: false,
                released: false,
            });
        }
        Self {
            controller,
            events: receiver,
            tasks,
        }
    }
    pub(super) async fn execute(&mut self) -> DropToken {
        let work = {
            let state = self.controller.state.lock().unwrap();
            let source = state.source.as_ref().unwrap();
            Work {
                request: InternalDropRequest {
                    id: source.id,
                    destination: InternalDropDestination::PaneRoot {
                        pane: PaneId::Right,
                        navigation: NavigationId(2),
                    },
                    intent: InternalDropIntent::Copy,
                    verification: TransferVerification::default(),
                },
                target: None,
                source_pane: source.pane,
                source_navigation: source.navigation,
                files: source.files.clone().unwrap(),
                captured: source.captured.clone(),
                cancel: source.cancel.clone(),
                host: state.host.clone().unwrap(),
                service: None,
            }
        };
        let state = self.controller.state.clone();
        let snapshots = self.controller.snapshots.clone();
        let events = self.controller.events.clone();
        let cancel = self.controller.cancel.clone();
        tokio::time::timeout(
            OBSERVE,
            tokio::task::spawn_blocking(move || {
                execute(work, &state, &snapshots, &events, &cancel);
            }),
        )
        .await
        .unwrap()
        .unwrap();
        match self.events.try_recv().unwrap() {
            DragDropEvent::DropAccepted {
                token,
                drag: Some(DragId(1)),
                ..
            } => token,
            event => panic!("actual internal drop did not acknowledge acceptance: {event:?}"),
        }
    }
    pub(super) async fn submit_backlog(&self) -> crate::retirement::Retirement {
        let lease = self.controller.backlog.lock().await.reserve().await;
        let retired = std::mem::take(&mut self.controller.state.lock().unwrap().retired);
        lease.retire(retired)
    }
    pub(super) async fn drain_old_gesture(&self) {
        let retired = self.submit_backlog().await;
        tokio::time::timeout(OBSERVE, retired)
            .await
            .unwrap()
            .unwrap();
        let state = self.controller.state.lock().unwrap();
        assert!(state.source.is_none() && state.retired.is_empty());
    }
    pub(super) async fn stop_forwarder(&mut self) {
        self.controller.cancel.cancel();
        tokio::time::timeout(OBSERVE, async {
            while let Some(result) = self.tasks.join_next().await {
                result.unwrap();
            }
        })
        .await
        .unwrap();
    }
    pub(super) async fn close(mut self) {
        self.controller.shutdown();
        let retired = self.submit_backlog().await;
        tokio::time::timeout(OBSERVE, retired)
            .await
            .unwrap()
            .unwrap();
        self.stop_forwarder().await;
    }
}

pub(super) struct Copies {
    pub(super) controller: copy_operations::Controller,
    pub(super) io: Arc<crate::transfer_leases::IoBudget>,
    events: mpsc::Receiver<Event>,
    tasks: JoinSet<()>,
    terminals: HashMap<OperationId, Arc<CopyProgress>>,
}
impl Copies {
    pub(super) fn new(service: Arc<TaggedService>) -> Self {
        let (sender, events) = mpsc::channel(256);
        let mut tasks = JoinSet::new();
        let io =
            crate::transfer_leases::IoBudget::new(Arc::new(midnight_work_budget::Gate::new(1)));
        let controller = copy_operations::Controller::new(
            crate::worker_retirement::ActorRetirement::testing(),
            Some(crate::transfer_composition::QualifiedTransferInstallation::audited(service)),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            crate::transfer_leases::namespace_gate(),
            io.clone(),
            &mut tasks,
        );
        Self {
            controller,
            io,
            events,
            tasks,
            terminals: HashMap::new(),
        }
    }
    fn remember(&mut self, event: Event) -> Option<Arc<CopyProgress>> {
        match event {
            Event::Copy(CopyEvent::Progress(progress)) => {
                if progress.state.is_terminal() {
                    self.terminals.insert(progress.id, progress.clone());
                }
                Some(progress)
            }
            Event::Copy(CopyEvent::Failure(failure)) => {
                self.controller.cancel(failure.id);
                None
            }
            Event::Copy(CopyEvent::Conflict(conflict)) => {
                self.controller.cancel(conflict.id);
                None
            }
            _ => None,
        }
    }
    pub(super) async fn wait_earlier(&mut self, id: u64) -> bool {
        tokio::time::timeout(OBSERVE, async {
            while let Some(event) = self.events.recv().await {
                if self.remember(event).is_some_and(|p| {
                    p.id == OperationId(id)
                        && p.wait_reason == Some(TransferWaitReason::EarlierTransfers)
                }) {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false)
    }
    pub(super) async fn terminal(&mut self, id: u64) -> Option<Arc<CopyProgress>> {
        tokio::time::timeout(OBSERVE, async {
            loop {
                if let Some(progress) = self.terminals.remove(&OperationId(id)) {
                    return Some(progress);
                }
                let event = self.events.recv().await?;
                self.remember(event);
            }
        })
        .await
        .ok()
        .flatten()
    }
    pub(super) async fn wait_capacity(&self, expected: usize) -> bool {
        wait_async(OBSERVE, || {
            self.controller.capacity().available_permits() == expected
        })
        .await
    }
    pub(super) async fn close(mut self) {
        self.controller.shutdown();
        tokio::time::timeout(OBSERVE, async {
            while let Some(result) = self.tasks.join_next().await {
                result.unwrap();
            }
        })
        .await
        .unwrap();
    }
}
