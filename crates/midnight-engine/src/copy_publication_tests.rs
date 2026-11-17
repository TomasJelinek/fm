//! Post-copy outcomes of regular files through the engine: a published file counts as
//! completed, an uncertain commit stops the job (the same policy as for links), and a
//! corrupted readback never publishes.
use super::*;
use std::{
    ffi::OsStr,
    io::{Read, Seek, SeekFrom, Write},
    sync::atomic::AtomicU8,
};

const PUBLISHED: u8 = 0;
const DURABILITY_UNCONFIRMED: u8 = 1;
const RECOVERY_REQUIRED: u8 = 2;
/// Flip one byte of the first independent readback of the first stage.
const CORRUPT_READBACK: u8 = 3;

/// Native service whose first publication reports the configured outcome.
struct Service {
    inner: midnight_platform::LocalFileSystem,
    outcome: Arc<AtomicU8>,
}
impl midnight_core::AuditedSynchronousTransferService for Service {}
impl TransferService for Service {
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
        Ok(Arc::new(Directory {
            inner: self.inner.prepare_transfer_destination(path, cancel)?,
            outcome: self.outcome.clone(),
        }))
    }
}
struct Directory {
    inner: Arc<dyn TransferDirectory>,
    outcome: Arc<AtomicU8>,
}
impl TransferDirectory for Directory {
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
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.inner.inspect(name, cancel)
    }
    fn prepare_target(
        &self,
        name: &OsStr,
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
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.create_directory(name, cancel)
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        Ok(Box::new(Stage {
            inner: self.inner.stage_file(cancel)?,
            outcome: self.outcome.clone(),
        }))
    }
}
struct Stage {
    inner: Box<dyn StagedTransferFile>,
    outcome: Arc<AtomicU8>,
}
impl Read for Stage {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(bytes)?;
        if count > 0
            && self
                .outcome
                .compare_exchange(
                    CORRUPT_READBACK,
                    PUBLISHED,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
        {
            bytes[0] ^= 0xff;
        }
        Ok(count)
    }
}
impl Write for Stage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Seek for Stage {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}
impl StagedTransferFile for Stage {
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferReadGuard>, FsError> {
        self.inner.rewind_for_validation(cancel)
    }
    fn revalidate_publication(&self, cancel: &Cancellation) -> Result<TransferIdentity, FsError> {
        self.inner.revalidate_publication(cancel)
    }
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.sync(cancel)
    }
    fn len(&self) -> Result<u64, FsError> {
        self.inner.len()
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        let publication = self.inner.publish_no_replace(name, cancel)?;
        let TransferPublication::Published(path) = publication else {
            return Ok(publication);
        };
        let error = FsError::from_io(
            "sync published parent",
            &path,
            io::Error::other("parent flush failed"),
        );
        Ok(match self.outcome.swap(PUBLISHED, Ordering::SeqCst) {
            DURABILITY_UNCONFIRMED => {
                TransferPublication::PublishedDurabilityUnconfirmed { path, error }
            }
            RECOVERY_REQUIRED => TransferPublication::RecoveryRequired {
                recovery: vec![TransferRecovery {
                    path: path.clone(),
                    identity: None,
                }],
                published: Some(path),
                error,
            },
            _ => TransferPublication::Published(path),
        })
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        self.inner.cleanup()
    }
}

async fn first_publication(outcome: u8) -> (Arc<CopyProgress>, PathBuf, tempfile::TempDir) {
    let temp = fixture();
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::write(&first, b"first published bytes").unwrap();
    fs::write(&second, b"never reached").unwrap();
    let mut h = Harness::new(Some(Arc::new(Service {
        inner: midnight_platform::LocalFileSystem::new(None),
        outcome: Arc::new(AtomicU8::new(outcome)),
    })));
    h.controller
        .enqueue(request(1, &[first, second], &destination))
        .await;
    let done = h.terminal(1).await;
    h.close().await;
    (done, destination, temp)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn published_file_with_unconfirmed_durability_counts_and_stops_the_job() {
    let (done, destination, _temp) = first_publication(DURABILITY_UNCONFIRMED).await;
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!(done.files_completed, 1);
    assert_eq!(done.completed_bytes, b"first published bytes".len() as u64);
    assert_eq!(done.failed, 1);
    assert!(done.errors[0].message.contains("durability is unconfirmed"));
    assert_eq!(
        fs::read(destination.join("first")).unwrap(),
        b"first published bytes"
    );
    assert!(
        !destination.join("second").exists(),
        "an uncertain commit stops the job"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_required_publication_keeps_its_facts_and_stops_the_job() {
    let (done, destination, _temp) = first_publication(RECOVERY_REQUIRED).await;
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!(done.files_completed, 1);
    assert_eq!(done.recovery_count, 1);
    assert_eq!(done.recovery_paths, vec![destination.join("first")]);
    assert_eq!(done.failed, 1);
    assert!(!destination.join("second").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_publications_continue_with_the_next_root() {
    let (done, destination, _temp) = first_publication(PUBLISHED).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.files_completed, 2);
    assert!(destination.join("second").exists());
}

/// A readback that differs from the source fails verification at the engine level: the
/// failure cannot be retried, Skip cleans the stage, and nothing is published.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corrupted_readback_fails_verification_cleans_the_stage_and_never_publishes() {
    let temp = fixture();
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let source = temp.path().join("verified");
    fs::write(&source, vec![7; 300_000]).unwrap();
    let mut h = Harness::new(Some(Arc::new(Service {
        inner: midnight_platform::LocalFileSystem::new(None),
        outcome: Arc::new(AtomicU8::new(CORRUPT_READBACK)),
    })));
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let failure = loop {
        match h.next().await {
            CopyEvent::Failure(failure) if failure.id == OperationId(1) => break failure,
            CopyEvent::Progress(progress) if progress.id == OperationId(1) => {
                assert!(!progress.state.is_terminal(), "{progress:?}")
            }
            _ => {}
        }
    };
    assert_eq!(
        failure.error.kind,
        io::ErrorKind::InvalidData,
        "{failure:?}"
    );
    assert!(!failure.can_retry && failure.can_skip);
    assert!(!destination.join("verified").exists());
    h.controller
        .decide(CopyDecision {
            id: OperationId(1),
            version: failure.version,
            choice: CopyChoice::Skip,
        })
        .await;
    let done = h.terminal(1).await;
    h.close().await;
    assert_eq!(done.state, CopyOperationState::Failed, "{done:?}");
    assert_eq!((done.files_completed, done.skipped, done.failed), (0, 1, 1));
    assert_eq!(
        fs::read_dir(&destination).unwrap().count(),
        0,
        "no stage or publication"
    );
    assert_eq!(fs::read(&source).unwrap(), vec![7; 300_000]);
}
