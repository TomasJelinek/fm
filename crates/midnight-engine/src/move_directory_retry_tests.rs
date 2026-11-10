use super::*;

struct RetryService {
    inner: midnight_platform::LocalFileSystem,
    enumerations: Arc<AtomicUsize>,
    removals: Arc<AtomicUsize>,
}
impl TransferService for RetryService {
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        let prepared = self.inner.prepare_move_source(path, cancel)?;
        Ok(PreparedTransferMove {
            source: Box::new(OnceEnumeration {
                inner: prepared.source,
                calls: self.enumerations.clone(),
            }),
            capture: Arc::new(RetryRemoval {
                inner: prepared.capture,
                calls: self.removals.clone(),
            }),
        })
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.prepare_move_destination(path, cancel)
    }
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
}
struct OnceEnumeration {
    inner: Box<dyn TransferSource>,
    calls: Arc<AtomicUsize>,
}
impl TransferSource for OnceEnumeration {
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
    fn next_move_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<PreparedTransferMove>, FsError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) != 0 {
            return Err(error(
                &self.entry().item.path,
                io::ErrorKind::NotConnected,
                "enumeration already ended before removal",
            ));
        }
        self.inner.next_move_child(cancel)
    }
}
struct RetryRemoval {
    inner: Arc<dyn TransferMoveCapture>,
    calls: Arc<AtomicUsize>,
}
impl TransferMoveCapture for RetryRemoval {
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.preflight_finalization(cancel)
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.inner.capture(cancel)
    }
    fn restore(&self) -> Result<(), FsError> {
        self.inner.restore()
    }
    fn remove_empty_directory(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryRemoval, FsError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(error(
                &self.entry().item.path,
                io::ErrorKind::PermissionDenied,
                "injected transient removal error",
            ));
        }
        self.inner.remove_empty_directory(cancel)
    }
}

#[test]
fn directory_removal_retry_does_not_reenter_completed_enumeration() {
    let (_temp, parent) = tree_fixture();
    let source = parent.join("empty");
    let destination = parent.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    let service = RetryService {
        inner: midnight_platform::LocalFileSystem::new(None),
        enumerations: Arc::new(AtomicUsize::new(0)),
        removals: Arc::new(AtomicUsize::new(0)),
    };
    let request = request(1, std::slice::from_ref(&source), &destination);
    let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
    let mut job =
        worker::Job::prepare_move(request, &service, Cancellation::default(), status).unwrap();
    let mut retried = false;
    for _ in 0..100 {
        match job.step() {
            worker::Step::Continue => {}
            worker::Step::Done => break,
            worker::Step::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
            worker::Step::Failure(failure) => {
                assert!(
                    !retried,
                    "removal retry must not invoke failed enumeration: {failure:?}"
                );
                assert_eq!(failure.stage, CopyErrorStage::RemovingSource);
                assert!(failure.can_retry);
                assert!(source.is_dir());
                assert!(job.accept_decision(CopyDecision {
                    id: failure.id,
                    version: failure.version,
                    choice: CopyChoice::Retry
                }));
                retried = true;
            }
        }
    }
    let done = job.finish();
    assert!(retried);
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(service.enumerations.load(Ordering::SeqCst), 1);
    assert_eq!(service.removals.load(Ordering::SeqCst), 2);
    assert!(!source.exists());
    assert!(destination.join("empty").is_dir());
    assert_eq!(done.failed, 0);
}
