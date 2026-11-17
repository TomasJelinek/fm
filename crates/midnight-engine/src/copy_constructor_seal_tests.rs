use super::*;
use crate::transfer_composition::QualifiedTransferInstallation;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Counts {
    source_drops: AtomicUsize,
    directory_drops: AtomicUsize,
    metadata_calls: AtomicUsize,
    bomb_drops: AtomicUsize,
}
struct Bomb(Arc<Counts>);
impl Drop for Bomb {
    fn drop(&mut self) {
        self.0.bomb_drops.fetch_add(1, Ordering::SeqCst);
        panic!("opaque metadata panic payload must not be dropped");
    }
}

// The same typed factory creates every source and directory used below. Its
// finite graph has only synchronous, in-memory owners and no deferred Drop work.
struct Service {
    counts: Arc<Counts>,
    panic_metadata: bool,
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
            panic_metadata: self.panic_metadata,
        }))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(Directory {
            path: path.to_owned(),
            counts: self.counts.clone(),
        }))
    }
}
struct Source {
    entry: TransferEntry,
    counts: Arc<Counts>,
    panic_metadata: bool,
}
impl TransferSource for Source {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        Some(self.entry.identity)
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        panic!("metadata preflight must not read content")
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        panic!("the fixture root is a file")
    }
    fn prepare_modified_time(
        &self,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferModifiedTimePin>, FsError> {
        self.counts.metadata_calls.fetch_add(1, Ordering::SeqCst);
        if self.panic_metadata {
            std::panic::panic_any(Bomb(self.counts.clone()));
        }
        Err(error(
            &self.entry.item.path,
            io::ErrorKind::PermissionDenied,
            "metadata refused",
        ))
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.counts.source_drops.fetch_add(1, Ordering::SeqCst);
    }
}
struct Directory {
    path: std::path::PathBuf,
    counts: Arc<Counts>,
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
    fn inspect(
        &self,
        _: &std::ffi::OsStr,
        _: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        Ok(None)
    }
    fn reject_source_ancestor(&self, _: TransferIdentity, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn create_directory(
        &self,
        _: &std::ffi::OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("metadata pin failed before directory creation")
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        panic!("metadata pin failed before stage creation")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        self.counts.directory_drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn identity(value: u8) -> TransferIdentity {
    TransferIdentity {
        device: 1,
        object: [value; 16],
    }
}

fn constructor_case(captured: bool, panic_metadata: bool) {
    let counts = Arc::new(Counts::default());
    let service = Arc::new(Service {
        counts: counts.clone(),
        panic_metadata,
    });
    let installation = QualifiedTransferInstallation::audited(service.clone());
    let root = std::env::current_dir().unwrap();
    let request = CopyRequest {
        id: OperationId(1),
        clipboard: FileClipboard {
            paths: vec![root.join("source")],
            intent: FileClipboardIntent::Copy,
        },
        destination: root.join("target"),
        verification: TransferVerification::FullSha256,
        start_policy: TransferStartPolicy::Immediate,
    };
    let status = Arc::new(Mutex::new(super::super::initial(&CopyInput::from(
        request.clone(),
    ))));
    let gate = Arc::new(midnight_work_budget::Gate::new(1));
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
    resources.attach_status(status.clone());
    resources
        .attach_io(
            gate.clone(),
            Arc::new(
                gate.try_acquire(midnight_work_budget::Admission::Shared)
                    .unwrap(),
            ),
        )
        .unwrap();
    let registry = crate::copy_namespace::Registry::new_with_io(gate.clone());
    let (claim, mut native) = registry.new_job(1).unwrap();
    native.qualify(installation.witness().unwrap());
    resources.install_namespace(claim, native).unwrap();
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let mut owned = domain
        .try_reserve()
        .unwrap()
        .own(ownership::Scope::new((), resources));
    let cancel = Cancellation::default();
    // The real Scope installs both the configured I/O witness and namespace
    // context; this is not a standalone unqualified constructor invocation.
    let result = ownership::run(&mut owned, |scope| {
        let resources = scope.resources_mut();
        let io = resources.real_io.as_ref().unwrap().clone();
        let claim = resources.namespace().unwrap();
        claim.plan_union(
            vec![
                (TransferNamespaceAccess::Read, None),
                (TransferNamespaceAccess::Write, None),
            ],
            io,
            &cancel,
        )?;
        assert!(claim.try_grant(&cancel)?);
        let result = if captured {
            let mut roots = VecDeque::new();
            roots.push_back(service.prepare_transfer_source(&request.clipboard.paths[0], &cancel)?);
            Job::prepare_sources(
                request.destination.clone(),
                roots,
                service.as_ref(),
                cancel.clone(),
                status.clone(),
                TransferMetadataPolicy::RequireModifiedTime,
            )
        } else {
            Job::prepare_with_metadata(
                request.clone(),
                service.as_ref(),
                cancel.clone(),
                status.clone(),
                TransferMetadataPolicy::RequireModifiedTime,
            )
        };
        result.map(|job| {
            let mut job = ownership::Armed::new(job);
            job.set_retirement_witness(installation.witness().unwrap());
            job
        })
    });
    let retired = ownership::retire_confirmed(owned);
    assert_eq!(counts.metadata_calls.load(Ordering::SeqCst), 1);
    assert_eq!(counts.bomb_drops.load(Ordering::SeqCst), 0);
    if panic_metadata {
        assert!(result.is_err());
        assert!(retired.is_err());
        assert_eq!(counts.source_drops.load(Ordering::SeqCst), 0);
        assert_eq!(counts.directory_drops.load(Ordering::SeqCst), 0);
        // Quarantine keeps native authority but returns the disk slot.
        assert_eq!(gate.snapshot().active_shared, 0);
        assert_eq!(namespace.available_permits(), 0);
        assert_eq!(capacity.available_permits(), 0);
        assert!(!waiting.is_finished());
        assert!(
            registry.new_job(1).is_err(),
            "the quarantined namespace record remains owned"
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let failed = runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(2),
                success.wait(&Cancellation::default(), Path::new("target")),
            )
            .await
        });
        assert!(
            failed
                .expect("SUCCESS must resolve despite retained FINISHED")
                .is_err()
        );
    } else {
        assert!(
            matches!(result, Ok(Err(error)) if error.kind == io::ErrorKind::PermissionDenied && error.message.contains("metadata refused"))
        );
        assert!(retired.is_ok());
        assert_eq!(counts.source_drops.load(Ordering::SeqCst), 1);
        assert_eq!(counts.directory_drops.load(Ordering::SeqCst), 1);
        assert_eq!(gate.snapshot().active_shared, 0);
        assert_eq!(namespace.available_permits(), 1);
        assert_eq!(capacity.available_permits(), 1);
        assert!(waiting.is_finished());
        assert!(
            registry.new_job(1).is_ok(),
            "ordinary refusal releases the old namespace record"
        );
    }
}

#[test]
fn native_metadata_panic_keeps_new_job_armed_and_all_authority_pending() {
    constructor_case(false, true);
}
#[test]
fn captured_metadata_panic_keeps_new_job_armed_and_all_authority_pending() {
    constructor_case(true, true);
}
#[test]
fn native_metadata_refusal_explicitly_drops_new_job_and_releases_authority() {
    constructor_case(false, false);
}
#[test]
fn captured_metadata_refusal_explicitly_drops_new_job_and_releases_authority() {
    constructor_case(true, false);
}
