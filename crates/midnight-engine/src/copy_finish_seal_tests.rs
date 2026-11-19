//! Captured Move restoration uses the same borrowed cleanup in both finish paths.
use super::*;
use std::{
    ffi::OsStr,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Clone, Copy)]
enum Restore {
    Refuse,
    Panic,
    Complete,
}
#[derive(Default)]
struct Calls {
    captured: AtomicBool,
    restored: AtomicUsize,
    dropped: AtomicUsize,
    bomb_drops: AtomicUsize,
    ticket: AtomicBool,
    order: Mutex<Vec<&'static str>>,
}
struct Bomb(Arc<Calls>);
impl Drop for Bomb {
    fn drop(&mut self) {
        self.0.bomb_drops.fetch_add(1, Ordering::SeqCst);
        panic!("opaque cleanup panic was destroyed");
    }
}
struct Capture {
    entry: TransferEntry,
    calls: Arc<Calls>,
    restore: Restore,
    panic_drop: bool,
}
impl TransferMoveCapture for Capture {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn capture(&self, _: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.calls.captured.store(true, Ordering::SeqCst);
        Ok(TransferMoveCaptureOutcome::Captured {
            source: self.entry.item.path.clone(),
            recovery: recovery(),
        })
    }
    fn restore(&self) -> Result<(), FsError> {
        assert!(self.calls.captured.load(Ordering::SeqCst));
        self.calls.restored.fetch_add(1, Ordering::SeqCst);
        self.calls.order.lock().unwrap().push("restore");
        match self.restore {
            Restore::Refuse => Err(FsError::from_io(
                "restore captured source",
                &recovery().path,
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "original retained in exact recovery slot",
                ),
            )),
            Restore::Panic => std::panic::panic_any(Bomb(self.calls.clone())),
            Restore::Complete => Ok(()),
        }
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn TransferMoveStageCleanup>> {
        if self.calls.ticket.swap(true, Ordering::SeqCst) {
            return None;
        }
        Some(Box::new(Ticket {
            path: PathBuf::from("held-stage"),
            calls: self.calls.clone(),
        }))
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.calls.dropped.fetch_add(1, Ordering::SeqCst);
        self.calls.order.lock().unwrap().push("capture-drop");
        if self.panic_drop {
            std::panic::panic_any(Bomb(self.calls.clone()));
        }
    }
}
struct Ticket {
    path: PathBuf,
    calls: Arc<Calls>,
}
impl TransferMoveStageCleanup for Ticket {
    fn path(&self) -> &Path {
        &self.path
    }
    fn cleanup(self: Box<Self>) -> Result<(), FsError> {
        assert_eq!(self.calls.dropped.load(Ordering::SeqCst), 1);
        self.calls.order.lock().unwrap().push("ticket-cleanup");
        Ok(())
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.calls.order.lock().unwrap().push("ticket-drop");
    }
}
struct Provider(Arc<Calls>);
impl crate::copy_nested_retirement::CheckedRetirement for Provider {
    fn retirement_is_checked(&self) -> bool {
        true
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.0.order.lock().unwrap().push("provider-drop");
    }
}
struct Directory;
impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        Path::new("destination")
    }
    fn identity(&self) -> TransferIdentity {
        identity(2)
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn inspect(&self, _: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
        panic!("cleanup never inspects destinations")
    }
    fn reject_source_ancestor(&self, _: TransferIdentity, _: &Cancellation) -> Result<(), FsError> {
        panic!("cleanup never prepares destinations")
    }
    fn create_directory(
        &self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("cleanup never creates directories")
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        panic!("cleanup never stages files")
    }
}
fn identity(value: u8) -> TransferIdentity {
    TransferIdentity {
        device: 1,
        object: [value; 16],
    }
}
fn recovery() -> TransferRecovery {
    TransferRecovery {
        path: PathBuf::from("held-stage/original"),
        identity: Some(identity(1)),
    }
}

// This factory creates the complete finite graph used below: an in-memory
// directory, capture and cleanup ticket. Injected panics are synchronous and
// must still be contained by the real Copy seal.
struct Service {
    calls: Arc<Calls>,
    restore: Restore,
    panic_drop: bool,
}
impl midnight_core::AuditedSynchronousTransferService for Service {}
impl TransferService for Service {
    fn prepare_transfer_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        panic!("cleanup fixture does not prepare a read source")
    }
    fn prepare_transfer_destination(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(Directory))
    }
}
impl Service {
    fn capture(&self) -> Capture {
        Capture {
            entry: TransferEntry {
                item: MutationItem {
                    path: PathBuf::from("source/original"),
                    kind: EntryKind::File,
                    metadata: EntryMetadata::default(),
                },
                identity: identity(1),
            },
            calls: self.calls.clone(),
            restore: self.restore,
            panic_drop: self.panic_drop,
        }
    }
}

struct Fixture {
    calls: Arc<Calls>,
    status: Status,
    io: Arc<midnight_work_budget::Gate>,
    namespace: Arc<Semaphore>,
    capacity: Arc<Semaphore>,
    pending: crate::copy_finished::Predecessors,
    old: crate::copy_fence::Predecessors,
    domain: crate::retirement::RetirementDomain,
}
impl Fixture {
    fn new(restore: Restore, panic_drop: bool) -> (Self, ownership::Owned<Job>) {
        let calls = Arc::new(Calls::default());
        let status = Arc::new(Mutex::new(CopyProgress::queued(
            OperationId(1),
            PathBuf::from("destination"),
            1,
            TransferIntent::Move,
        )));
        let cancel = Cancellation::default();
        let service = Arc::new(Service {
            calls: calls.clone(),
            restore,
            panic_drop,
        });
        let destination = service
            .prepare_transfer_destination(Path::new("destination"), &cancel)
            .unwrap();
        let mut job =
            Job::from_sources(destination, VecDeque::new(), cancel.clone(), status.clone());
        let capture = service.capture();
        let witness = crate::transfer_composition::QualifiedTransferInstallation::audited(service)
            .witness()
            .unwrap();
        job.set_retirement_witness(witness);
        assert!(matches!(
            capture.capture(&cancel).unwrap(),
            TransferMoveCaptureOutcome::Captured { .. }
        ));
        job.move_work = Some(MoveWork {
            captures: vec![Arc::new(capture)],
            link_captures: HashMap::new(),
            metadata_policy: TransferMetadataPolicy::ContentOnly,
            stages: HashMap::new(),
            proofs: Vec::new(),
            phase: Phase::Halted,
            finalizer: None,
            active: Some((PathBuf::from("source/original"), recovery())),
            reading: false,
            buffer: Vec::new(),
            admitted: 1,
            refresh_parents: Vec::new(),
            failed_phase: None,
            retryable_failure: false,
        });
        let io = Arc::new(midnight_work_budget::Gate::new(1));
        let namespace = Arc::new(Semaphore::new(1));
        let capacity = Arc::new(Semaphore::new(1));
        let mut registry = crate::copy_finished::Registry::default();
        let (finished, predecessors) = registry.register().unwrap();
        let (_, pending) = registry.register().unwrap();
        let fence = crate::copy_fence::CopyFence::default();
        let receipt = fence.register();
        let old = fence.snapshot();
        let mut resources = ownership::Resources::admission(
            receipt,
            finished,
            capacity.clone().try_acquire_owned().unwrap(),
            predecessors,
        );
        resources.mutation = Some(namespace.clone().try_acquire_owned().unwrap());
        let permit = Arc::new(
            io.acquire(midnight_work_budget::Admission::Shared, || false)
                .unwrap(),
        );
        resources.attach_io(io.clone(), permit).unwrap();
        resources.retain_checked_provider(Box::new(Provider(calls.clone())));
        let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
        let owned = domain
            .try_reserve()
            .unwrap()
            .own(ownership::Scope::job(job, resources));
        (
            Self {
                calls,
                status,
                io,
                namespace,
                capacity,
                pending,
                old,
                domain,
            },
            owned,
        )
    }
    fn held(&self) {
        self.retained(1);
    }
    /// Quarantine keeps native authority, the namespace share and admission capacity but
    /// returns the disk slot, an admission counter that would otherwise block all I/O.
    fn quarantined(&self) {
        self.retained(0);
    }
    fn retained(&self, disk_slots: u8) {
        assert_eq!(self.io.snapshot().active_shared, disk_slots);
        assert_eq!(self.namespace.available_permits(), 0);
        assert_eq!(self.capacity.available_permits(), 0);
        assert!(!self.pending.is_finished());
        assert_eq!(self.calls.bomb_drops.load(Ordering::SeqCst), 0);
    }
    fn closed(&self) {
        assert_eq!(self.io.snapshot().active_shared, 0);
        assert_eq!(self.namespace.available_permits(), 1);
        assert_eq!(self.capacity.available_permits(), 1);
        assert!(self.pending.is_finished());
    }
}

#[test]
fn captured_move_restore_bomb_retains_native_owner_without_retry_or_drop() {
    if ownership::seal_tests::child_case(concat!(
        module_path!(),
        "::captured_move_restore_bomb_retains_native_owner_without_retry_or_drop"
    )) {
        return;
    }
    let (fixture, owned) = Fixture::new(Restore::Panic, false);
    assert!(matches!(
        ownership::finish(owned),
        Err(ownership::Quarantined)
    ));
    fixture.quarantined();
    assert_eq!(fixture.calls.restored.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.calls.dropped.load(Ordering::SeqCst), 0);
    assert_eq!(*fixture.calls.order.lock().unwrap(), ["restore"]);
}

#[test]
fn captured_move_destructor_bomb_keeps_cleanup_ticket_and_finished_pending() {
    if ownership::seal_tests::child_case(concat!(
        module_path!(),
        "::captured_move_destructor_bomb_keeps_cleanup_ticket_and_finished_pending"
    )) {
        return;
    }
    let (fixture, owned) = Fixture::new(Restore::Complete, true);
    assert!(matches!(
        ownership::finish(owned),
        Err(ownership::Quarantined)
    ));
    fixture.quarantined();
    assert_eq!(fixture.calls.restored.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.calls.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(
        *fixture.calls.order.lock().unwrap(),
        ["restore", "capture-drop"]
    );
}

#[test]
fn captured_move_refusal_preserves_exact_residual_but_finishes_after_drop() {
    let (fixture, owned) = Fixture::new(Restore::Refuse, false);
    let progress = ownership::finish(owned).unwrap();
    assert_eq!(progress.state, CopyOperationState::Failed);
    assert!(progress.errors.iter().any(
        |error| error.path == recovery().path && error.kind == io::ErrorKind::PermissionDenied
    ));
    assert_eq!(progress.source_residual_count, 1);
    assert_eq!(progress.source_residuals[0].temporary.path, recovery().path);
    fixture.closed();
    assert_eq!(
        *fixture.calls.order.lock().unwrap(),
        ["restore", "capture-drop", "provider-drop"]
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    assert!(
        runtime
            .block_on(
                fixture
                    .old
                    .wait(&Cancellation::default(), Path::new("later"))
            )
            .is_err()
    );
}

#[test]
fn borrowed_finish_is_idempotent_and_tickets_close_before_finished() {
    let (fixture, mut owned) = Fixture::new(Restore::Complete, false);
    ownership::run(&mut owned, |scope| {
        scope.finish_in_place();
        // Idempotent: the order below records each owner's cleanup exactly once.
        scope.finish_in_place();
    })
    .unwrap();
    fixture.held();
    assert_eq!(fixture.calls.restored.load(Ordering::SeqCst), 1);
    assert_eq!(
        *fixture.calls.order.lock().unwrap(),
        ["restore", "capture-drop", "ticket-cleanup", "ticket-drop"]
    );
    let progress = ownership::finish(owned).unwrap();
    assert_eq!(progress.state, CopyOperationState::Completed);
    fixture.closed();
    assert_eq!(fixture.calls.restored.load(Ordering::SeqCst), 1);
}

#[test]
fn forced_scope_retirement_restores_move_before_cancelled_status_and_finished() {
    let (fixture, owned) = Fixture::new(Restore::Complete, false);
    let retired = owned.retire();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), retired)
            .await
            .unwrap()
            .unwrap();
    });
    fixture.closed();
    assert_eq!(
        snapshot(&fixture.status).state,
        CopyOperationState::Cancelled
    );
    assert_eq!(
        *fixture.calls.order.lock().unwrap(),
        [
            "restore",
            "capture-drop",
            "ticket-cleanup",
            "ticket-drop",
            "provider-drop"
        ]
    );
    assert!(fixture.domain.try_reserve().is_ok());
}

struct LateLinkFailure {
    status: Status,
    diagnostics: Arc<link_job::Diagnostics>,
}
// Both fields retain host-only progress/diagnostic metadata. Drop records the
// complete failure synchronously and owns no native or deferred cleanup tail.
impl crate::copy_nested_retirement::CheckedRetirement for LateLinkFailure {
    fn retirement_is_checked(&self) -> bool {
        true
    }
}
impl Drop for LateLinkFailure {
    fn drop(&mut self) {
        assert!(!matches!(
            snapshot(&self.status).state,
            CopyOperationState::Completed
                | CopyOperationState::CompletedWithErrors
                | CopyOperationState::Cancelled
                | CopyOperationState::Failed
        ));
        self.diagnostics.record(TransferLinkCleanupError {
            error: FsError::from_io(
                "close link stage",
                Path::new("held-link"),
                io::ErrorKind::PermissionDenied.into(),
            ),
            recovery: vec![TransferRecovery {
                path: PathBuf::from("held-link"),
                identity: None,
            }],
            cleanup_unconfirmed: false,
        });
    }
}

#[test]
fn native_drop_link_diagnostic_is_drained_before_final_result_and_finished() {
    let (fixture, mut owned) = Fixture::new(Restore::Complete, false);
    let diagnostics = owned.link_retirement.diagnostics();
    owned
        .resources_mut()
        .retain_native(Box::new(LateLinkFailure {
            status: fixture.status.clone(),
            diagnostics,
        }));
    let progress = ownership::finish(owned).unwrap();
    assert_eq!(progress.state, CopyOperationState::Failed);
    assert!(
        progress
            .errors
            .iter()
            .any(|error| error.path == Path::new("held-link"))
    );
    assert_eq!(progress.recovery_paths, [PathBuf::from("held-link")]);
    fixture.closed();
}
