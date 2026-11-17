//! Namespace changes refused by granted peers. The refused step mutated nothing, so it is
//! repeated automatically once a refusing peer changes; the refused job's own turns never
//! end that wait. Quarantined peers and private-name collisions remain ordinary failures.
use super::*;

/// Refuses the first stage creations as a native adapter does when a granted peer's
/// namespace (opaque, latched or intersecting) refuses its private reservation: `record`
/// is what the refused reservation recorded, `kind` what the adapter reported.
struct RefusingService {
    inner: midnight_platform::LocalFileSystem,
    refusals: Arc<AtomicUsize>,
    record: fn(),
    kind: io::ErrorKind,
}
impl RefusingService {
    fn new(refusals: usize, record: fn(), kind: io::ErrorKind) -> (Arc<AtomicUsize>, Self) {
        let left = Arc::new(AtomicUsize::new(refusals));
        let service = Self {
            inner: midnight_platform::LocalFileSystem::new(None),
            refusals: left.clone(),
            record,
            kind,
        };
        (left, service)
    }
}
impl midnight_core::AuditedSynchronousTransferService for RefusingService {}
impl TransferService for RefusingService {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.inner.prepare_transfer_source(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(RefusingDirectory {
            inner: self.inner.prepare_transfer_destination(path, cancel)?,
            refusals: self.refusals.clone(),
            record: self.record,
            kind: self.kind,
        }))
    }
}
struct RefusingDirectory {
    inner: Arc<dyn TransferDirectory>,
    refusals: Arc<AtomicUsize>,
    record: fn(),
    kind: io::ErrorKind,
}
impl TransferDirectory for RefusingDirectory {
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn identity(&self) -> TransferIdentity {
        self.inner.identity()
    }
    fn capabilities(&self) -> TransferCapabilities {
        self.inner.capabilities()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn inspect(
        &self,
        name: &std::ffi::OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.inner.inspect(name, cancel)
    }
    fn prepare_target(
        &self,
        name: &std::ffi::OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        self.inner.prepare_target(name, cancel)
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.reject_source_ancestor(source, cancel)
    }
    fn create_directory(
        &self,
        name: &std::ffi::OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.create_directory(name, cancel)
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        if self
            .refusals
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            (self.record)();
            return Err(FsError::from_io(
                "create transfer stage",
                self.path(),
                io::Error::new(self.kind, "a granted peer refused the name"),
            ));
        }
        self.inner.stage_file(cancel)
    }
}
fn no_record() {}

/// The terminal progress of every job in `ids`, in that order; any prompt fails the test.
async fn terminals_without_prompt(h: &mut Harness, ids: &[u64]) -> Vec<Arc<CopyProgress>> {
    let mut terminals = vec![None; ids.len()];
    while terminals.iter().any(Option::is_none) {
        match h.next().await {
            CopyEvent::Failure(failure) => {
                panic!("a peer refusal must not ask the user: {failure:?}")
            }
            CopyEvent::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
            CopyEvent::Progress(progress) if progress.state.is_terminal() => {
                if let Some(index) = ids.iter().position(|id| progress.id == OperationId(*id)) {
                    assert!(terminals[index].is_none(), "duplicate terminal event");
                    terminals[index] = Some(progress);
                }
            }
            _ => {}
        }
    }
    terminals.into_iter().flatten().collect()
}
async fn terminal_without_prompt(h: &mut Harness, id: u64) -> Arc<CopyProgress> {
    terminals_without_prompt(h, &[id]).await.remove(0)
}

/// A namespace change refused by a granted peer before any native mutation is retried
/// automatically after the registry changes: no failure prompt, no recorded error. The
/// decision rests on the recorded refusal, whatever error kind the adapter reported.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn peer_refused_stage_creation_retries_without_a_failure_prompt() {
    for kind in [
        io::ErrorKind::WouldBlock,
        io::ErrorKind::AlreadyExists,
        io::ErrorKind::InvalidInput,
    ] {
        let temp = fixture();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::write(&source, b"retried after the peer").unwrap();
        fs::create_dir(&destination).unwrap();
        let (refusals, service) =
            RefusingService::new(2, crate::copy_namespace::refuse_for_test, kind);
        let mut h = Harness::new(Some(Arc::new(service)));
        h.controller
            .enqueue(request(1, std::slice::from_ref(&source), &destination))
            .await;
        let done = terminal_without_prompt(&mut h, 1).await;
        assert_eq!(
            done.state,
            CopyOperationState::Completed,
            "{kind:?}: {done:?}"
        );
        assert!(done.errors.is_empty());
        assert_eq!(refusals.load(Ordering::SeqCst), 0);
        assert_eq!(
            fs::read(destination.join("source")).unwrap(),
            b"retried after the peer"
        );
        h.close().await;
    }
}

/// A stage name that already exists without any peer refusal is not a late collision of
/// the user's final name: it fails once with a prompt instead of being retried silently
/// and immediately in a loop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_name_already_exists_is_an_ordinary_failure_not_a_silent_retry() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, b"never staged").unwrap();
    fs::create_dir(&destination).unwrap();
    let (left, service) = RefusingService::new(1_000, no_record, io::ErrorKind::AlreadyExists);
    let mut h = Harness::new(Some(Arc::new(service)));
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let failure = loop {
        match h.next().await {
            CopyEvent::Failure(failure) if failure.id == OperationId(1) => break failure,
            CopyEvent::Progress(progress) => assert!(!progress.state.is_terminal()),
            _ => {}
        }
    };
    assert_eq!(failure.error.kind, io::ErrorKind::AlreadyExists);
    assert_eq!(
        left.load(Ordering::SeqCst),
        999,
        "exactly one stage attempt"
    );
    h.controller
        .decide(CopyDecision {
            id: failure.id,
            version: failure.version,
            choice: CopyChoice::Cancel,
        })
        .await;
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Cancelled);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    h.close().await;
}

/// A refusal by a quarantined peer never clears: the entry fails at once with the
/// actionable restart error and cannot be retried (Skip and Cancel remain).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quarantined_peer_refusal_fails_with_the_restart_error() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, b"blocked by quarantine").unwrap();
    fs::create_dir(&destination).unwrap();
    let (_, service) = RefusingService::new(
        1,
        crate::copy_namespace::refuse_quarantined_for_test,
        io::ErrorKind::WouldBlock,
    );
    let mut h = Harness::new(Some(Arc::new(service)));
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let failure = loop {
        match h.next().await {
            CopyEvent::Failure(failure) if failure.id == OperationId(1) => break failure,
            CopyEvent::Progress(progress) => assert!(!progress.state.is_terminal()),
            _ => {}
        }
    };
    assert!(
        crate::copy_namespace::is_blocked_by_quarantine(&failure.error),
        "{failure:?}"
    );
    assert!(failure.error.message.contains("Restart"));
    assert!(!failure.can_retry && failure.can_skip);
    h.controller
        .decide(CopyDecision {
            id: failure.id,
            version: failure.version,
            choice: CopyChoice::Cancel,
        })
        .await;
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Cancelled);
    h.close().await;
}

/// Wraps the cross-device fixture's Move captures: the first source removals and empty
/// directory removals are refused as the native adapters refuse them when a granted
/// peer's namespace refuses the change (recorded refusal, nothing mutated).
struct RefusingMoveService {
    inner: moves::CrossDeviceService,
    removals: Arc<AtomicUsize>,
    directories: Arc<AtomicUsize>,
    finalizers: Arc<AtomicUsize>,
}
impl RefusingMoveService {
    fn new(removals: usize, directories: usize) -> Self {
        Self {
            inner: moves::CrossDeviceService(midnight_platform::LocalFileSystem::new(None)),
            removals: Arc::new(AtomicUsize::new(removals)),
            directories: Arc::new(AtomicUsize::new(directories)),
            finalizers: Arc::new(AtomicUsize::new(0)),
        }
    }
}
// The native-delegating fixture retains only synchronous authority and counters.
impl midnight_core::AuditedSynchronousTransferService for RefusingMoveService {}
impl TransferService for RefusingMoveService {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.inner.prepare_transfer_source(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.prepare_transfer_destination(path, cancel)
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.prepare_move_destination(path, cancel)
    }
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        let prepared = self.inner.prepare_move_source(path, cancel)?;
        Ok(PreparedTransferMove {
            source: prepared.source,
            capture: Arc::new(RefusingCapture {
                inner: prepared.capture,
                removals: self.removals.clone(),
                directories: self.directories.clone(),
                finalizers: self.finalizers.clone(),
                receipt: Mutex::new(None),
            }),
        })
    }
}
fn refused(left: &AtomicUsize) -> bool {
    left.try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
        left.checked_sub(1)
    })
    .is_ok()
}
fn refusal(path: &Path) -> FsError {
    crate::copy_namespace::refuse_for_test();
    FsError::from_io(
        "transfer namespace",
        path,
        io::Error::new(
            io::ErrorKind::WouldBlock,
            "a granted peer refused the change",
        ),
    )
}
struct RefusingCapture {
    inner: Arc<dyn TransferMoveCapture>,
    removals: Arc<AtomicUsize>,
    directories: Arc<AtomicUsize>,
    finalizers: Arc<AtomicUsize>,
    receipt: Mutex<Option<(PathBuf, TransferRecovery)>>,
}
impl TransferMoveCapture for RefusingCapture {
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn try_native_move(
        &self,
        destination: &dyn TransferDirectory,
        name: &std::ffi::OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        self.inner
            .try_native_move(destination, name, replace, cancel)
    }
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.preflight_finalization(cancel)
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        self.finalizers.fetch_add(1, Ordering::SeqCst);
        let (source, recovery) = self.receipt.lock().unwrap().clone().unwrap();
        Ok(Box::new(RefusingFinalizer {
            inner: self.inner.prepare_finalization(cancel)?,
            removals: self.removals.clone(),
            source,
            recovery,
        }))
    }
    fn remove_empty_directory(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryRemoval, FsError> {
        if refused(&self.directories) {
            return Err(refusal(&self.entry().item.path));
        }
        self.inner.remove_empty_directory(cancel)
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        let outcome = self.inner.capture(cancel)?;
        if let TransferMoveCaptureOutcome::Captured { source, recovery } = &outcome {
            *self.receipt.lock().unwrap() = Some((source.clone(), recovery.clone()));
        }
        Ok(outcome)
    }
    fn restore(&self) -> Result<(), FsError> {
        self.inner.restore()
    }
    fn restoration_completed(&self) -> bool {
        self.inner.restoration_completed()
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn TransferMoveStageCleanup>> {
        self.inner.take_stage_cleanup()
    }
}
struct RefusingFinalizer {
    inner: Box<dyn TransferMoveFinalizer>,
    removals: Arc<AtomicUsize>,
    source: PathBuf,
    recovery: TransferRecovery,
}
impl TransferMoveFinalizer for RefusingFinalizer {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn finalize(&mut self, cancel: &Cancellation) -> TransferMoveFinalizeOutcome {
        if refused(&self.removals) {
            return TransferMoveFinalizeOutcome::Retained {
                source: self.source.clone(),
                recovery: self.recovery.clone(),
                error: refusal(&self.source),
            };
        }
        self.inner.finalize(cancel)
    }
}

/// A Move's verified source removal refused by a granted peer is repeated automatically
/// (no prompt): the ordinary Retry rebuilds the finalizer and rehashes both retained
/// objects before the source is removed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn peer_refused_move_source_removal_rehashes_and_retries_without_a_prompt() {
    let (_temp, parent) = moves::protected_fixture();
    let source = parent.join("moved");
    let destination = parent.join("destination");
    let bytes: Vec<_> = (0..300_011).map(|index| (index % 253) as u8).collect();
    fs::write(&source, &bytes).unwrap();
    fs::create_dir(&destination).unwrap();
    let service = RefusingMoveService::new(1, 0);
    let (removals, finalizers) = (service.removals.clone(), service.finalizers.clone());
    let mut h = Harness::new(Some(Arc::new(service)));
    h.controller
        .enqueue_move(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let done = terminal_without_prompt(&mut h, 1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert!(done.errors.is_empty(), "{done:?}");
    assert_eq!(removals.load(Ordering::SeqCst), 0);
    assert_eq!(
        finalizers.load(Ordering::SeqCst),
        2,
        "Retry rebuilt the finalizer"
    );
    // Before and After proofs, plus the repeated After proof (source and destination).
    assert_eq!(done.validation_bytes, 6 * bytes.len() as u64, "{done:?}");
    assert_eq!(done.source_files_removed, 1);
    assert!(!source.exists());
    assert_eq!(fs::read(destination.join("moved")).unwrap(), bytes);
    h.close().await;
}

/// A Move's empty source directory removal refused by a granted peer repeats exactly
/// that removal (the exhausted frame is kept) instead of asking the user.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn peer_refused_move_directory_removal_retries_without_a_prompt() {
    let (_temp, parent) = moves::protected_fixture();
    let root = parent.join("tree");
    let destination = parent.join("destination");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("child"), b"moved child").unwrap();
    fs::create_dir(&destination).unwrap();
    let service = RefusingMoveService::new(0, 1);
    let directories = service.directories.clone();
    let mut h = Harness::new(Some(Arc::new(service)));
    h.controller
        .enqueue_move(request(1, std::slice::from_ref(&root), &destination))
        .await;
    let done = terminal_without_prompt(&mut h, 1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert!(done.errors.is_empty(), "{done:?}");
    assert_eq!(directories.load(Ordering::SeqCst), 0);
    assert!(!root.exists());
    assert_eq!(
        fs::read(destination.join("tree").join("child")).unwrap(),
        b"moved child"
    );
    h.close().await;
}

/// Job 1 (`slow` source) blocks in its first read; job 2's destination (`counted`)
/// counts stage creations and holds its first target probe until released.
struct PeerService {
    inner: midnight_platform::LocalFileSystem,
    slow_entered: Arc<Notify>,
    slow_released: Arc<AtomicBool>,
    probe_entered: Arc<Notify>,
    probe_released: Arc<AtomicBool>,
    stages: Arc<AtomicUsize>,
}
impl midnight_core::AuditedSynchronousTransferService for PeerService {}
impl TransferService for PeerService {
    // The destination's exact native footprint: without it both jobs would be opaque.
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.prepare_checksum_directory(path, cancel)
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        let inner = self.inner.prepare_transfer_source(path, cancel)?;
        Ok(if path.ends_with("slow") {
            Box::new(SlowSource {
                inner,
                entered: self.slow_entered.clone(),
                released: self.slow_released.clone(),
            })
        } else {
            inner
        })
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        let inner = self.inner.prepare_transfer_destination(path, cancel)?;
        Ok(if path.ends_with("counted") {
            Arc::new(GatedDirectory {
                inner,
                entered: self.probe_entered.clone(),
                released: self.probe_released.clone(),
                stages: self.stages.clone(),
            })
        } else {
            inner
        })
    }
}
/// Blocks its first read; forwards its exact native footprint, so the job is not opaque.
struct SlowSource {
    inner: Box<dyn TransferSource>,
    entered: Arc<Notify>,
    released: Arc<AtomicBool>,
}
impl TransferSource for SlowSource {
    fn native_namespace_scope(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        self.inner.native_namespace_scope(cancel)
    }
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.entered.notify_one();
        while !self.released.load(Ordering::Acquire) {
            std::thread::park_timeout(Duration::from_millis(2));
        }
        self.inner.read_chunk(buffer, cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.inner.next_child(cancel)
    }
}
/// Forwards its exact native footprint; holds the first target probe and counts stages.
struct GatedDirectory {
    inner: Arc<dyn TransferDirectory>,
    entered: Arc<Notify>,
    released: Arc<AtomicBool>,
    stages: Arc<AtomicUsize>,
}
impl TransferDirectory for GatedDirectory {
    fn native_child_namespace_scope(
        &self,
        name: &std::ffi::OsStr,
        kind: EntryKind,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        self.inner.native_child_namespace_scope(name, kind, cancel)
    }
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn identity(&self) -> TransferIdentity {
        self.inner.identity()
    }
    fn capabilities(&self) -> TransferCapabilities {
        self.inner.capabilities()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn inspect(
        &self,
        name: &std::ffi::OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.inner.inspect(name, cancel)
    }
    fn prepare_target(
        &self,
        name: &std::ffi::OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        self.entered.notify_one();
        while !self.released.load(Ordering::Acquire) {
            std::thread::park_timeout(Duration::from_millis(2));
        }
        self.inner.prepare_target(name, cancel)
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.reject_source_ancestor(source, cancel)
    }
    fn create_directory(
        &self,
        name: &std::ffi::OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.create_directory(name, cancel)
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.stages.fetch_add(1, Ordering::SeqCst);
        self.inner.stage_file(cancel)
    }
}

/// A job whose stage creation is refused by a persistently opaque (latched) granted peer
/// waits for that peer's removal: it does not rerun its step every retry spacing, and
/// it completes without a prompt once the peer is gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistently_opaque_peer_refusal_waits_without_rerunning_the_step() {
    let temp = fixture();
    let slow = temp.path().join("slow");
    let fast = temp.path().join("fast");
    let first_destination = temp.path().join("first");
    let counted = temp.path().join("counted");
    fs::write(&slow, vec![5; 300_000]).unwrap();
    fs::write(&fast, b"waits for the peer").unwrap();
    fs::create_dir(&first_destination).unwrap();
    fs::create_dir(&counted).unwrap();
    let service = PeerService {
        inner: midnight_platform::LocalFileSystem::new(None),
        slow_entered: Arc::new(Notify::new()),
        slow_released: Arc::new(AtomicBool::new(false)),
        probe_entered: Arc::new(Notify::new()),
        probe_released: Arc::new(AtomicBool::new(false)),
        stages: Arc::new(AtomicUsize::new(0)),
    };
    let (slow_entered, slow_released) =
        (service.slow_entered.clone(), service.slow_released.clone());
    let (probe_entered, probe_released) = (
        service.probe_entered.clone(),
        service.probe_released.clone(),
    );
    let stages = service.stages.clone();
    let _release_slow = ReleaseOnDrop(slow_released.clone());
    let _release_probe = ReleaseOnDrop(probe_released.clone());
    let mut h = Harness::with_slots(Some(Arc::new(service)), 2);
    h.controller
        .enqueue(request(1, std::slice::from_ref(&slow), &first_destination))
        .await;
    tokio::time::timeout(Duration::from_secs(4), slow_entered.notified())
        .await
        .expect("the first job runs");
    h.controller
        .enqueue(request(2, std::slice::from_ref(&fast), &counted))
        .await;
    tokio::time::timeout(Duration::from_secs(4), probe_entered.notified())
        .await
        .expect("the disjoint second job is granted too");
    let granted = h.controller.namespace().granted_for_test();
    assert_eq!(granted.len(), 2, "{granted:?}");
    h.controller.namespace().latch_opaque_for_test(granted[0]);
    probe_released.store(true, Ordering::Release);
    h.wait_status(2, |progress| {
        progress.wait_reason == Some(TransferWaitReason::Resources)
            && stages.load(Ordering::SeqCst) == 1
    })
    .await;
    // Ten retry spacings: a wait ended by the job's own refused turn would retry here.
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        stages.load(Ordering::SeqCst),
        1,
        "a refused job reruns its step only when a refusing peer changes"
    );
    slow_released.store(true, Ordering::Release);
    let done = terminals_without_prompt(&mut h, &[1, 2]).await;
    assert_eq!(
        done[0].state,
        CopyOperationState::Completed,
        "{:?}",
        done[0]
    );
    assert_eq!(
        done[1].state,
        CopyOperationState::Completed,
        "{:?}",
        done[1]
    );
    assert_eq!(
        stages.load(Ordering::SeqCst),
        2,
        "one retry after the latched peer was removed"
    );
    assert_eq!(
        fs::read(counted.join("fast")).unwrap(),
        b"waits for the peer"
    );
    h.close().await;
}
