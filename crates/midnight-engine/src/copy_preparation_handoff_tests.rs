use super::*;
use crate::transfer_composition::QualifiedTransferInstallation;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Default)]
struct Counts {
    read_directory_drops: AtomicUsize,
    write_directory_drops: AtomicUsize,
    source_drops: AtomicUsize,
    captured_releases: AtomicUsize,
    captured_drops: AtomicUsize,
    query_calls: AtomicUsize,
    writable_created: AtomicUsize,
    bomb_drops: AtomicUsize,
}
struct Bomb(Arc<Counts>);
impl Drop for Bomb {
    fn drop(&mut self) {
        self.0.bomb_drops.fetch_add(1, Ordering::SeqCst);
        panic!("opaque panic payload must be forgotten");
    }
}

// This concrete factory creates only the finite synchronous source/directory
// graph below. No native handles, foreign retirement queues or erased factory
// witness are borrowed by the fixture.
struct Service {
    counts: Arc<Counts>,
    panic_read_directory: bool,
}
impl midnight_core::AuditedSynchronousTransferService for Service {}
impl TransferService for Service {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Ok(Box::new(Source {
            entry: TransferEntry {
                item: MutationItem {
                    path: path.to_owned(),
                    kind: EntryKind::File,
                    metadata: EntryMetadata {
                        size: Some(0),
                        ..Default::default()
                    },
                },
                identity: identity(1),
            },
            counts: self.counts.clone(),
        }))
    }
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(Directory {
            path: path.to_owned(),
            counts: self.counts.clone(),
            readonly: true,
            panic_drop: self.panic_read_directory,
        }))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.counts.writable_created.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Directory {
            path: path.to_owned(),
            counts: self.counts.clone(),
            readonly: false,
            panic_drop: false,
        }))
    }
}
struct Source {
    entry: TransferEntry,
    counts: Arc<Counts>,
}
impl TransferSource for Source {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        panic!("preparation does not read file content")
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        panic!("fixture root is a file")
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.counts.source_drops.fetch_add(1, Ordering::SeqCst);
    }
}
struct Directory {
    path: PathBuf,
    counts: Arc<Counts>,
    readonly: bool,
    panic_drop: bool,
}
impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        &self.path
    }
    fn identity(&self) -> TransferIdentity {
        identity(2)
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn inspect(&self, _: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
        Ok(None)
    }
    fn reject_source_ancestor(&self, _: TransferIdentity, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn create_directory(
        &self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("preparation does not create directories")
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        panic!("preparation does not stage files")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        if self.readonly {
            self.counts
                .read_directory_drops
                .fetch_add(1, Ordering::SeqCst);
            if self.panic_drop {
                std::panic::panic_any(Bomb(self.counts.clone()));
            }
        } else {
            self.counts
                .write_directory_drops
                .fetch_add(1, Ordering::SeqCst);
        }
    }
}
fn identity(value: u8) -> TransferIdentity {
    TransferIdentity {
        device: 1,
        object: [value; 16],
    }
}
fn input() -> CopyInput {
    let root = std::env::current_dir().unwrap();
    CopyInput::from(CopyRequest {
        id: OperationId(1),
        clipboard: FileClipboard {
            paths: vec![root.join("source")],
            intent: FileClipboardIntent::Copy,
        },
        destination: root.join("target"),
        verification: TransferVerification::FullSha256,
        start_policy: TransferStartPolicy::Immediate,
    })
}
struct Authority {
    io: Arc<midnight_work_budget::Gate>,
    namespace: Arc<Semaphore>,
    capacity: Arc<Semaphore>,
    finished: crate::copy_finished::Predecessors,
    success: crate::copy_fence::Predecessors,
    domain: crate::retirement::RetirementDomain,
    _registry: crate::copy_namespace::Registry,
}
impl Authority {
    fn new(status: Status) -> (Self, ownership::Resources) {
        let io = Arc::new(midnight_work_budget::Gate::new(1));
        let namespace = Arc::new(Semaphore::new(1));
        let capacity = Arc::new(Semaphore::new(1));
        let mut finished = crate::copy_finished::Registry::default();
        let (owner, predecessors) = finished.register().unwrap();
        let (_, waiting) = finished.register().unwrap();
        let fence = crate::copy_fence::CopyFence::default();
        let receipt = fence.register();
        let success = fence.snapshot();
        let mut resources = ownership::Resources::admission(
            receipt,
            owner,
            capacity.clone().try_acquire_owned().unwrap(),
            predecessors,
        );
        resources.mutation = Some(namespace.clone().try_acquire_owned().unwrap());
        resources.attach_status(status);
        resources
            .attach_io(
                io.clone(),
                Arc::new(
                    io.acquire(midnight_work_budget::Admission::Shared, || false)
                        .unwrap(),
                ),
            )
            .unwrap();
        let registry = crate::copy_namespace::Registry::new_with_io(io.clone());
        let (claim, native) = registry.new_job(1).unwrap();
        resources.install_namespace(claim, native).unwrap();
        let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
        (
            Self {
                io,
                namespace,
                capacity,
                finished: waiting,
                success,
                domain,
                _registry: registry,
            },
            resources,
        )
    }
    fn assert_quarantined(&self) {
        assert_eq!(self.io.snapshot().active_shared, 1);
        assert_eq!(self.namespace.available_permits(), 0);
        assert_eq!(self.capacity.available_permits(), 0);
        assert!(!self.finished.is_finished());
        assert!(
            matches!(self._registry.new_job(1), Err(error) if error.kind == io::ErrorKind::InvalidInput),
            "the original logical namespace record must remain reserved"
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let cancel = Cancellation::default();
        let failed = runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(2),
                self.success.wait(&cancel, Path::new("target")),
            )
            .await
        });
        assert!(
            failed
                .expect("SUCCESS must fail independently of pending FINISHED")
                .is_err()
        );
    }
}

#[test]
fn readonly_destination_drop_panic_keeps_returned_job_native_owners_armed() {
    let counts = Arc::new(Counts::default());
    let input = input();
    let status = Arc::new(Mutex::new(super::super::initial(&input)));
    let installation = QualifiedTransferInstallation::audited(Arc::new(Service {
        counts: counts.clone(),
        panic_read_directory: true,
    }));
    let preparation = Preparation::qualified(input, installation);
    let (authority, resources) = Authority::new(status.clone());
    let mut owned = authority
        .domain
        .try_reserve()
        .unwrap()
        .own(ownership::Scope::new(preparation, resources));
    let result = ownership::run(&mut owned, |scope| {
        let cancel = Cancellation::default();
        let endpoints = scope.pin_endpoints(&cancel, &status)?;
        let resources = scope.resources_mut();
        let io = resources.real_io.as_ref().unwrap().clone();
        let claim = resources.namespace().unwrap();
        claim.plan_union(endpoints, io, &cancel)?;
        assert!(claim.try_grant(&cancel)?);
        scope
            .attempt(cancel, status.clone())
            .map(ownership::Armed::new)
    });
    assert!(result.is_err());
    assert!(ownership::retire_confirmed(owned).is_err());
    assert_eq!(counts.writable_created.load(Ordering::SeqCst), 1);
    assert_eq!(counts.read_directory_drops.load(Ordering::SeqCst), 1);
    assert_eq!(counts.write_directory_drops.load(Ordering::SeqCst), 0);
    assert_eq!(counts.source_drops.load(Ordering::SeqCst), 0);
    assert_eq!(counts.bomb_drops.load(Ordering::SeqCst), 0);
    authority.assert_quarantined();
}

#[derive(Debug)]
struct Captured(Arc<Counts>);
impl CapturedTransferSources for Captured {
    fn intent(&self) -> TransferIntent {
        TransferIntent::Copy
    }
    fn root_count(&self) -> usize {
        1
    }
    fn metadata_available(&self) -> bool {
        false
    }
    fn prepares_audited_native_children(&self) -> bool {
        self.0.query_calls.fetch_add(1, Ordering::SeqCst);
        std::panic::panic_any(Bomb(self.0.clone()));
    }
    fn prepare(
        &self,
        _: &dyn TransferService,
        _: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        panic!("query refusal precedes source preparation")
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        self.0.captured_releases.fetch_add(1, Ordering::SeqCst);
        drop(Arc::into_inner(self));
        Ok(())
    }
}
impl Drop for Captured {
    fn drop(&mut self) {
        self.0.captured_drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn constructor_query_panic_retains_consumed_captured_input_before_outer_catch() {
    let counts = Arc::new(Counts::default());
    let input = CopyInput::from(CapturedTransferRequest {
        id: OperationId(1),
        sources: Arc::new(Captured(counts.clone())),
        destination: std::env::current_dir().unwrap().join("target"),
        metadata: TransferMetadataPolicy::ContentOnly,
        verification: TransferVerification::FullSha256,
        start_policy: TransferStartPolicy::Immediate,
    });
    let status = Arc::new(Mutex::new(super::super::initial(&input)));
    let (authority, resources) = Authority::new(status);
    let owned = authority
        .domain
        .try_reserve()
        .unwrap()
        .own(ownership::Scope::new(input, resources));
    let installation = QualifiedTransferInstallation::audited(Arc::new(Service {
        counts: counts.clone(),
        panic_read_directory: false,
    }));
    let result = ownership::map(owned, |input| Preparation::qualified(input, installation));
    assert!(result.is_err());
    assert_eq!(counts.query_calls.load(Ordering::SeqCst), 1);
    assert_eq!(counts.captured_releases.load(Ordering::SeqCst), 0);
    assert_eq!(counts.captured_drops.load(Ordering::SeqCst), 0);
    assert_eq!(counts.bomb_drops.load(Ordering::SeqCst), 0);
    authority.assert_quarantined();
}
