use super::*;
use midnight_core::DiscoveryState;
use std::sync::Condvar;

#[derive(Default)]
struct GateState {
    allowed: usize,
    release: bool,
}

struct DiscoveryGate {
    state: Mutex<GateState>,
    changed: Condvar,
    ready: Notify,
    observed: mpsc::UnboundedSender<(usize, MutationItem)>,
    returned: AtomicBool,
    dropped: AtomicBool,
}

impl DiscoveryGate {
    fn wait(&self, index: usize) -> bool {
        let mut state = self.state.lock().unwrap();
        while !state.release && state.allowed <= index {
            state = self.changed.wait(state).unwrap();
        }
        !state.release
    }

    fn advance(&self, allowed: usize) {
        self.state.lock().unwrap().allowed = allowed;
        self.changed.notify_all();
    }

    fn release(&self) {
        self.state.lock().unwrap().release = true;
        self.changed.notify_all();
    }
}

struct ReleaseDiscovery(Arc<DiscoveryGate>);
impl Drop for ReleaseDiscovery {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct DiscoverySource {
    inner: Box<dyn TransferSource>,
    gate: Arc<DiscoveryGate>,
}

impl Drop for DiscoverySource {
    fn drop(&mut self) {
        self.gate.dropped.store(true, Ordering::Release);
    }
}

impl TransferSource for DiscoverySource {
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }

    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.inner.read_chunk(buffer, cancel)
    }

    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.inner.next_child(cancel)
    }

    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        let result = (|| {
            let mut items = Vec::new();
            self.inner
                .discover(cancel, &mut |item| items.push(item.clone()))?;
            // Stable callback order for the owned directory plus its two regular files.
            items.sort_by(|left, right| left.path.cmp(&right.path));
            assert_eq!(items.len(), 3);
            self.gate.ready.notify_one();
            for (index, item) in items.into_iter().enumerate() {
                // Deliberately retain an in-flight call after cancellation, as an OS
                // call may do. Only the test owner may release this blocking point.
                if !self.gate.wait(index) {
                    return Err(FsError::from_io(
                        "discover fixture",
                        &self.entry().item.path,
                        std::io::ErrorKind::Interrupted.into(),
                    ));
                }
                self.revalidate(cancel)?;
                visit(&item);
                self.gate.observed.send((index + 1, item)).unwrap();
            }
            self.gate.wait(3);
            self.revalidate(cancel)
        })();
        self.gate.returned.store(true, Ordering::Release);
        result
    }
}

struct DiscoveryService {
    inner: midnight_platform::LocalFileSystem,
    gate: Arc<DiscoveryGate>,
}

// This native-delegating fixture retains only synchronous authority and host counters.
impl midnight_core::AuditedSynchronousTransferService for DiscoveryService {}
impl TransferService for DiscoveryService {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Ok(Box::new(DiscoverySource {
            inner: self.inner.prepare_transfer_source(path, cancel)?,
            gate: self.gate.clone(),
        }))
    }

    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.prepare_transfer_destination(path, cancel)
    }
}

struct DiscoveryFixture {
    // Unblock native ownership before dropping the Harness or temporary filesystem.
    _release: ReleaseDiscovery,
    h: Harness,
    gate: Arc<DiscoveryGate>,
    observed: mpsc::UnboundedReceiver<(usize, MutationItem)>,
    root: PathBuf,
    destination: PathBuf,
    _temporary: tempfile::TempDir,
}

impl DiscoveryFixture {
    async fn start() -> Self {
        let temporary = fixture();
        let root = temporary.path().join("discovery source");
        let destination = temporary.path().join("destination");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(root.join("first.txt"), b"abc").unwrap();
        fs::write(root.join("second.txt"), b"12345").unwrap();
        let (sender, observed) = mpsc::unbounded_channel();
        let gate = Arc::new(DiscoveryGate {
            state: Mutex::default(),
            changed: Condvar::new(),
            ready: Notify::new(),
            observed: sender,
            returned: AtomicBool::new(false),
            dropped: AtomicBool::new(false),
        });
        let mut h = Harness::new(Some(Arc::new(DiscoveryService {
            inner: midnight_platform::LocalFileSystem::new(None),
            gate: gate.clone(),
        })));
        let release = ReleaseDiscovery(gate.clone());
        h.controller
            .enqueue(request(1, std::slice::from_ref(&root), &destination))
            .await;
        loop {
            match h.next().await {
                CopyEvent::Progress(progress)
                    if progress.state == CopyOperationState::Preparing =>
                {
                    break;
                }
                CopyEvent::Progress(progress) => {
                    assert!(!progress.state.is_terminal(), "{progress:?}")
                }
                event => panic!("unexpected preparation event: {event:?}"),
            }
        }
        tokio::time::timeout(Duration::from_secs(4), gate.ready.notified())
            .await
            .expect("source never entered controlled discovery");
        Self {
            _release: release,
            h,
            gate,
            observed,
            root,
            destination,
            _temporary: temporary,
        }
    }

    async fn report(&mut self, count: usize) -> MutationItem {
        // Cross the production discovery callback's 50ms coalescing interval.
        // Synchronization itself is the explicit gate/acknowledgement, not this delay.
        tokio::time::sleep(Duration::from_millis(75)).await;
        self.gate.advance(count);
        let (actual, item) = tokio::time::timeout(Duration::from_secs(4), self.observed.recv())
            .await
            .unwrap()
            .expect("discovery callback exited without an acknowledgement");
        assert_eq!(actual, count);
        item
    }

    async fn scanning(&mut self, files: u64, bytes: u64, path: &Path) -> Arc<CopyProgress> {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                match self.h.next().await {
                    CopyEvent::Progress(progress) => {
                        assert!(!progress.state.is_terminal(), "{progress:?}");
                        if progress.discovery.state != DiscoveryState::Scanning
                            || progress.discovery.files != files
                            || progress.current_path.as_deref() != Some(path)
                        {
                            continue;
                        }
                        assert_eq!(progress.state, CopyOperationState::Preparing);
                        assert_eq!(progress.discovery.directories, 1);
                        assert_eq!(progress.discovery.bytes, Some(bytes));
                        assert_eq!((progress.copied_bytes, progress.verified_bytes), (0, 0));
                        assert_eq!(progress.throughput.transfer_per_second, 0.0);
                        assert_eq!(progress.throughput.verification_per_second, 0.0);
                        assert!(!self.gate.returned.load(Ordering::Acquire));
                        assert_eq!(fs::read_dir(&self.destination).unwrap().count(), 0);
                        return progress;
                    }
                    event => panic!("discovery unexpectedly left preparation: {event:?}"),
                }
            }
        })
        .await
        .expect("changed discovery never reached the Controller event stream")
    }

    async fn cancel_and_finish(&mut self) {
        self.h.controller.cancel(OperationId(1));
        self.gate.release();
        self.cancelled_terminal().await;
        assert!(self.gate.returned.load(Ordering::Acquire));
        assert!(self.gate.dropped.load(Ordering::Acquire));
        assert_eq!(fs::read_dir(&self.destination).unwrap().count(), 0);
    }

    async fn cancelled_terminal(&mut self) {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                match self.h.next().await {
                    CopyEvent::Progress(progress) => {
                        assert_eq!(progress.id, OperationId(1));
                        if progress.state.is_terminal() {
                            assert_eq!(progress.state, CopyOperationState::Cancelled);
                            assert_eq!(progress.failed, 0, "{progress:?}");
                            return;
                        }
                    }
                    event => panic!("unexpected discovery cancellation event: {event:?}"),
                }
            }
        })
        .await
        .expect("cancelled discovery never finished retirement");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preparing_discovery_publishes_successive_counts_and_paths_without_throughput() {
    let mut fixture = DiscoveryFixture::start().await;
    let root = fixture.report(1).await;
    assert_eq!(root.path, fixture.root);
    fixture.scanning(0, 0, &root.path).await;
    let first = fixture.report(2).await;
    assert_eq!(first.path, fixture.root.join("first.txt"));
    fixture.scanning(1, 3, &first.path).await;
    let second = fixture.report(3).await;
    assert_eq!(second.path, fixture.root.join("second.txt"));
    fixture.scanning(2, 8, &second.path).await;
    fixture.cancel_and_finish().await;
    fixture.h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_outbox_retries_latest_discovery_without_another_callback() {
    let mut fixture = DiscoveryFixture::start().await;
    let root = fixture.report(1).await;
    let stale = fixture.scanning(0, 0, &root.path).await;
    let sender = fixture.h.controller.outbox.sender.clone();
    let mut filled = 0;
    loop {
        match sender.try_send(Event::Copy(CopyEvent::Progress(stale.clone()))) {
            Ok(()) => filled += 1,
            Err(mpsc::error::TrySendError::Full(_)) => break,
            Err(mpsc::error::TrySendError::Closed(_)) => panic!("outbox closed"),
        }
    }
    assert!(filled > 0);
    assert_eq!(sender.capacity(), 0);
    let first = fixture.report(2).await;
    let second = fixture.report(3).await;
    assert_ne!(first.path, second.path);
    // Give the real publisher multiple opportunities to encounter the full outbox.
    tokio::time::sleep(crate::operation_throughput::CADENCE * 3).await;
    assert_eq!(sender.capacity(), 0);
    // The native call remains blocked and generates no further callback. A lost
    // dirty update cannot be repaired by a later native progress mutation here.
    fixture.scanning(2, 8, &second.path).await;
    fixture.cancel_and_finish().await;
    fixture.h.close().await;
}

async fn interrupted_discovery_retains_source_until_actual_return(shutdown: bool) {
    let mut fixture = DiscoveryFixture::start().await;
    fixture.report(1).await;
    if shutdown {
        fixture.h.controller.shutdown();
    } else {
        fixture.h.controller.cancel(OperationId(1));
    }
    let premature = tokio::time::timeout(Duration::from_millis(100), async {
        while let Some(event) = fixture.h.events.recv().await {
            if let Event::Copy(CopyEvent::Progress(progress)) = event
                && progress.state.is_terminal()
            {
                return true;
            }
        }
        panic!("event stream closed before discovery returned");
    })
    .await;
    assert!(
        premature.is_err(),
        "terminal progress preceded actual blocking return"
    );
    assert!(!fixture.gate.returned.load(Ordering::Acquire));
    assert!(!fixture.gate.dropped.load(Ordering::Acquire));
    assert_eq!(
        fixture.h.controller.capacity().available_permits(),
        MAX_JOBS - 1
    );
    fixture.gate.release();
    if shutdown {
        fixture.h.wait_shutdown().await;
    } else {
        fixture.cancelled_terminal().await;
    }
    fixture.h.wait_capacity(MAX_JOBS).await;
    assert!(fixture.gate.returned.load(Ordering::Acquire));
    assert!(fixture.gate.dropped.load(Ordering::Acquire));
    assert_eq!(
        fixture.h.controller.capacity().available_permits(),
        MAX_JOBS
    );
    assert_eq!(fs::read(fixture.root.join("first.txt")).unwrap(), b"abc");
    assert_eq!(fs::read_dir(&fixture.destination).unwrap().count(), 0);
    fixture.h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_discovery_keeps_source_owned_until_blocking_return() {
    interrupted_discovery_retains_source_until_actual_return(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_discovery_keeps_source_owned_until_blocking_return() {
    interrupted_discovery_retains_source_until_actual_return(true).await;
}
