use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct Counts {
    source_opens: AtomicUsize,
    destination_opens: AtomicUsize,
    move_opens: AtomicUsize,
    move_destinations: AtomicUsize,
    source_drops: AtomicUsize,
    capture_drops: AtomicUsize,
    fail_second: AtomicBool,
    invalid_source: AtomicBool,
    invalid_destination: AtomicBool,
    invalid_capture: AtomicBool,
    retry_offset: AtomicUsize,
    provider: Mutex<Option<std::sync::Weak<PreparationProviderLease>>>,
    authority_drop_context: Mutex<Vec<(&'static str, bool)>>,
}

impl Counts {
    fn record_authority_drop(&self, authority: &'static str) {
        if let Some(provider) = self.provider.lock().unwrap().as_ref() {
            self.authority_drop_context
                .lock()
                .unwrap()
                .push((authority, provider.upgrade().is_some()));
        }
    }
}

struct Service(Arc<Counts>);

impl TransferService for Service {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.0.source_opens.fetch_add(1, Ordering::SeqCst);
        if path.file_name() == Some(OsStr::new("second"))
            && self.0.fail_second.swap(false, Ordering::SeqCst)
        {
            return Err(error(
                path,
                io::ErrorKind::NotConnected,
                "Device is unavailable.",
            ));
        }
        Ok(Box::new(Source {
            entry: entry(path),
            counts: self.0.clone(),
        }))
    }

    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.destination_opens.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Directory {
            path: path.to_owned(),
            counts: self.0.clone(),
        }))
    }

    fn prepare_move_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        self.0.move_opens.fetch_add(1, Ordering::SeqCst);
        if path.file_name() == Some(OsStr::new("second"))
            && self.0.fail_second.swap(false, Ordering::SeqCst)
        {
            return Err(error(
                path,
                io::ErrorKind::NotConnected,
                "Move device is unavailable.",
            ));
        }
        Ok(PreparedTransferMove {
            source: Box::new(Source {
                entry: entry(path),
                counts: self.0.clone(),
            }),
            capture: Arc::new(Capture {
                entry: entry(path),
                counts: self.0.clone(),
            }),
        })
    }

    fn prepare_move_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.move_destinations.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Directory {
            path: path.to_owned(),
            counts: self.0.clone(),
        }))
    }
}

fn entry(path: &Path) -> TransferEntry {
    TransferEntry {
        item: MutationItem {
            path: path.to_owned(),
            kind: EntryKind::File,
            metadata: EntryMetadata {
                size: Some(0),
                ..EntryMetadata::default()
            },
        },
        identity: TransferIdentity {
            device: 1,
            object: [if path.file_name() == Some(OsStr::new("first")) {
                1
            } else {
                2
            }; 16],
        },
    }
}

struct Source {
    entry: TransferEntry,
    counts: Arc<Counts>,
}

impl Drop for Source {
    fn drop(&mut self) {
        self.counts.record_authority_drop("source");
        self.counts.source_drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl TransferSource for Source {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }

    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        if self.counts.invalid_source.load(Ordering::SeqCst) {
            Err(error(
                &self.entry.item.path,
                io::ErrorKind::InvalidData,
                "Source changed.",
            ))
        } else {
            Ok(())
        }
    }

    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        panic!("preparation must not read source bytes")
    }

    fn supports_read_retry(&self) -> bool {
        true
    }

    fn retry_read_from(&mut self, offset: u64, _: &Cancellation) -> Result<(), FsError> {
        self.counts
            .retry_offset
            .store(offset as usize, Ordering::SeqCst);
        Ok(())
    }

    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        panic!("preparation must not enumerate a source")
    }
}

struct Capture {
    entry: TransferEntry,
    counts: Arc<Counts>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.counts.record_authority_drop("capture");
        self.counts.capture_drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl TransferMoveCapture for Capture {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }

    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        if self.counts.invalid_capture.load(Ordering::SeqCst) {
            Err(error(
                &self.entry.item.path,
                io::ErrorKind::InvalidData,
                "Capture changed.",
            ))
        } else {
            Ok(())
        }
    }

    fn capture(&self, _: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        panic!("preparation must not capture a source")
    }
}

struct Directory {
    path: PathBuf,
    counts: Arc<Counts>,
}

impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        &self.path
    }

    fn identity(&self) -> TransferIdentity {
        TransferIdentity {
            device: 1,
            object: [3; 16],
        }
    }

    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        if self.counts.invalid_destination.load(Ordering::SeqCst) {
            Err(error(
                &self.path,
                io::ErrorKind::InvalidData,
                "Destination changed.",
            ))
        } else {
            Ok(())
        }
    }

    fn inspect(&self, _: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
        panic!("preparation must not inspect a file destination")
    }

    fn reject_source_ancestor(&self, _: TransferIdentity, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }

    fn create_directory(
        &self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("preparation must not create a published directory")
    }

    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        panic!("ContentOnly Copy preparation does not need a stage")
    }
}

fn setup() -> (Preparation, Status, Arc<Counts>) {
    let root = std::env::current_dir().unwrap();
    let input = CopyInput::from(CopyRequest {
        verification: midnight_core::TransferVerification::default(),
        id: OperationId(1),
        clipboard: FileClipboard {
            paths: vec![root.join("first"), root.join("second")],
            intent: FileClipboardIntent::Copy,
        },
        destination: root.join("destination"),
    });
    let status = Arc::new(Mutex::new(super::super::initial(&input)));
    let counts = Arc::new(Counts::default());
    counts.fail_second.store(true, Ordering::SeqCst);
    let preparation = Preparation::new(input, Arc::new(Service(counts.clone())));
    (preparation, status, counts)
}

#[test]
fn retained_preparation_and_admitted_jobs_can_move_between_backend_workers() {
    fn assert_send<T: Send>() {}
    assert_send::<Preparation>();
    assert_send::<worker::Job>();
}

#[test]
fn failed_preparation_retains_prior_sources_and_destination_for_retry() {
    let (mut preparation, status, counts) = setup();
    assert!(
        preparation
            .attempt(Cancellation::default(), status.clone())
            .is_err()
    );
    assert_eq!(counts.source_drops.load(Ordering::SeqCst), 0);
    let job = preparation
        .attempt(Cancellation::default(), status.clone())
        .unwrap();
    assert_eq!(counts.source_opens.load(Ordering::SeqCst), 3);
    assert_eq!(counts.destination_opens.load(Ordering::SeqCst), 1);
    assert_eq!(status.lock().unwrap().requested_roots, 2);
    drop(job);
    // Successful preparation must release its extra cache owners even if the
    // Preparation value has not yet been dropped by its worker.
    assert_eq!(counts.source_drops.load(Ordering::SeqCst), 2);
    assert!(
        preparation
            .attempt(Cancellation::default(), status)
            .is_err()
    );
    assert_eq!(counts.source_opens.load(Ordering::SeqCst), 3);
}

#[test]
fn retry_refuses_changed_held_source_instead_of_reopening_the_path() {
    let (mut preparation, status, counts) = setup();
    assert!(
        preparation
            .attempt(Cancellation::default(), status.clone())
            .is_err()
    );
    counts.invalid_source.store(true, Ordering::SeqCst);
    let failure = preparation
        .attempt(Cancellation::default(), status)
        .err()
        .unwrap();
    assert_eq!(failure.kind, io::ErrorKind::InvalidData);
    assert_eq!(counts.source_opens.load(Ordering::SeqCst), 2);
    assert_eq!(counts.destination_opens.load(Ordering::SeqCst), 1);
}

#[test]
fn retry_refuses_changed_held_destination() {
    let (mut preparation, status, counts) = setup();
    assert!(
        preparation
            .attempt(Cancellation::default(), status.clone())
            .is_err()
    );
    counts.invalid_destination.store(true, Ordering::SeqCst);
    let failure = preparation
        .attempt(Cancellation::default(), status)
        .err()
        .unwrap();
    assert_eq!(failure.kind, io::ErrorKind::InvalidData);
    assert_eq!(counts.source_opens.load(Ordering::SeqCst), 2);
    assert_eq!(counts.destination_opens.load(Ordering::SeqCst), 1);
}

#[test]
fn cancellation_retains_pins_until_worker_drops_preparation() {
    let (mut preparation, status, counts) = setup();
    assert!(
        preparation
            .attempt(Cancellation::default(), status.clone())
            .is_err()
    );
    let cancel = Cancellation::default();
    cancel.cancel();
    let failure = preparation.attempt(cancel, status).err().unwrap();
    assert_eq!(failure.kind, io::ErrorKind::Interrupted);
    assert_eq!(counts.source_opens.load(Ordering::SeqCst), 2);
    assert_eq!(counts.source_drops.load(Ordering::SeqCst), 0);
    drop(preparation);
    assert_eq!(counts.source_drops.load(Ordering::SeqCst), 1);
}

#[test]
fn source_wrapper_forwards_read_retry_to_the_original_source() {
    let counts = Arc::new(Counts::default());
    let retained = RetainedService::new(Arc::new(Service(counts.clone())));
    let path = std::env::current_dir().unwrap().join("first");
    let cancel = Cancellation::default();
    let mut source = retained.prepare_transfer_source(&path, &cancel).unwrap();
    assert!(source.supports_read_retry());
    source.retry_read_from(7, &cancel).unwrap();
    assert_eq!(counts.retry_offset.load(Ordering::SeqCst), 7);
    assert_eq!(counts.source_opens.load(Ordering::SeqCst), 1);
}

#[test]
fn copy_and_move_authorities_are_distinct_and_move_capture_is_retained() {
    let counts = Arc::new(Counts::default());
    let mut retained = RetainedService::new(Arc::new(Service(counts.clone())));
    let path = std::env::current_dir().unwrap().join("first");
    let cancel = Cancellation::default();
    let copy = retained.prepare_transfer_source(&path, &cancel).unwrap();
    let first = retained.prepare_move_source(&path, &cancel).unwrap();
    let second = retained.prepare_move_source(&path, &cancel).unwrap();
    assert!(Arc::ptr_eq(&first.capture, &second.capture));
    assert_eq!(counts.source_opens.load(Ordering::SeqCst), 1);
    assert_eq!(counts.move_opens.load(Ordering::SeqCst), 1);
    let copy_destination = retained
        .prepare_transfer_destination(&path, &cancel)
        .unwrap();
    let move_destination = retained.prepare_move_destination(&path, &cancel).unwrap();
    assert!(!Arc::ptr_eq(&copy_destination, &move_destination));
    assert_eq!(counts.destination_opens.load(Ordering::SeqCst), 1);
    assert_eq!(counts.move_destinations.load(Ordering::SeqCst), 1);
    counts.invalid_capture.store(true, Ordering::SeqCst);
    assert!(retained.prepare_move_source(&path, &cancel).is_err());
    assert_eq!(counts.move_opens.load(Ordering::SeqCst), 1);
    retained.release();
    drop(copy);
    drop(first);
    drop(second);
    assert_eq!(counts.source_drops.load(Ordering::SeqCst), 2);
    assert_eq!(counts.capture_drops.load(Ordering::SeqCst), 1);
}

#[derive(Debug)]
struct CapturedPaths(Vec<PathBuf>);
impl CapturedTransferSources for CapturedPaths {
    fn intent(&self) -> TransferIntent {
        TransferIntent::Copy
    }
    fn root_count(&self) -> usize {
        self.0.len()
    }
    fn metadata_available(&self) -> bool {
        true
    }
    fn prepare(
        &self,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        self.0
            .iter()
            .map(|path| service.prepare_transfer_source(path, cancel))
            .collect::<Result<_, _>>()
            .map(PreparedCapturedTransfer::Copy)
    }
}
fn captured_setup() -> (Preparation, Status, Arc<Counts>) {
    let root = std::env::current_dir().unwrap();
    let input = CopyInput::from(CapturedTransferRequest {
        verification: midnight_core::TransferVerification::default(),
        id: OperationId(1),
        sources: Arc::new(CapturedPaths(vec![root.join("first"), root.join("second")])),
        destination: root.join("destination"),
        metadata: TransferMetadataPolicy::ContentOnly,
    });
    let status = Arc::new(Mutex::new(super::super::initial(&input)));
    let counts = Arc::new(Counts::default());
    counts.fail_second.store(true, Ordering::SeqCst);
    (
        Preparation::new(input, Arc::new(Service(counts.clone()))),
        status,
        counts,
    )
}
#[test]
fn captured_retry_retains_native_identity_and_forwards_read_retry() {
    let (mut preparation, status, counts) = captured_setup();
    assert!(
        preparation
            .attempt(Cancellation::default(), status.clone())
            .is_err()
    );
    assert_eq!(counts.source_opens.load(Ordering::SeqCst), 2);
    assert_eq!(counts.source_drops.load(Ordering::SeqCst), 0);
    counts.invalid_source.store(true, Ordering::SeqCst);
    assert!(
        preparation
            .attempt(Cancellation::default(), status.clone())
            .is_err()
    );
    assert_eq!(counts.source_opens.load(Ordering::SeqCst), 2);
    counts.invalid_source.store(false, Ordering::SeqCst);
    let job = preparation
        .attempt(Cancellation::default(), status)
        .unwrap();
    assert_eq!(counts.source_opens.load(Ordering::SeqCst), 3);
    drop(job);
    assert_eq!(counts.source_drops.load(Ordering::SeqCst), 2);
}

#[derive(Debug)]
struct CapturedMovePaths(Vec<PathBuf>);
impl CapturedTransferSources for CapturedMovePaths {
    fn intent(&self) -> TransferIntent {
        TransferIntent::Move
    }
    fn root_count(&self) -> usize {
        self.0.len()
    }
    fn metadata_available(&self) -> bool {
        false
    }
    fn prepare(
        &self,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        self.0
            .iter()
            .map(|path| service.prepare_move_source(path, cancel))
            .collect::<Result<_, _>>()
            .map(PreparedCapturedTransfer::Move)
    }
}
#[test]
fn captured_move_retry_retains_original_mutation_pin_after_later_open_fails() {
    let root = std::env::current_dir().unwrap();
    let input = CopyInput::from(CapturedTransferRequest {
        verification: midnight_core::TransferVerification::default(),
        id: OperationId(1),
        destination: root.join("destination"),
        metadata: TransferMetadataPolicy::ContentOnly,
        sources: Arc::new(CapturedMovePaths(vec![
            root.join("first"),
            root.join("second"),
        ])),
    });
    let status = Arc::new(Mutex::new(super::super::initial(&input)));
    let counts = Arc::new(Counts::default());
    counts.fail_second.store(true, Ordering::SeqCst);
    let mut preparation = Preparation::new(input, Arc::new(Service(counts.clone())));
    assert!(
        preparation
            .attempt(Cancellation::default(), status.clone())
            .is_err()
    );
    assert_eq!(counts.move_opens.load(Ordering::SeqCst), 2);
    assert_eq!(counts.capture_drops.load(Ordering::SeqCst), 0);
    counts.invalid_capture.store(true, Ordering::SeqCst);
    assert!(
        preparation
            .attempt(Cancellation::default(), status)
            .is_err()
    );
    assert_eq!(
        counts.move_opens.load(Ordering::SeqCst),
        2,
        "Retry must not reopen a replacement native name"
    );
    assert_eq!(counts.move_destinations.load(Ordering::SeqCst), 0);
    drop(preparation);
    assert_eq!(counts.capture_drops.load(Ordering::SeqCst), 1);
    assert_eq!(counts.source_drops.load(Ordering::SeqCst), 1);
}

#[derive(Debug)]
struct PreparationProviderLease {
    _permit: OwnedSemaphorePermit,
}

#[derive(Debug)]
struct LeasedCapturedMovePaths {
    paths: CapturedMovePaths,
    _provider: Arc<PreparationProviderLease>,
}

impl CapturedTransferSources for LeasedCapturedMovePaths {
    fn intent(&self) -> TransferIntent {
        self.paths.intent()
    }
    fn root_count(&self) -> usize {
        self.paths.root_count()
    }
    fn metadata_available(&self) -> bool {
        self.paths.metadata_available()
    }
    fn prepare(
        &self,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        self.paths.prepare(service, cancel)
    }
}

#[test]
fn failed_captured_move_preparation_drops_native_authorities_before_provider_lease() {
    let root = std::env::current_dir().unwrap();
    let capacity = Arc::new(Semaphore::new(1));
    let provider = Arc::new(PreparationProviderLease {
        _permit: capacity.clone().try_acquire_owned().unwrap(),
    });
    let counts = Arc::new(Counts::default());
    *counts.provider.lock().unwrap() = Some(Arc::downgrade(&provider));
    counts.fail_second.store(true, Ordering::SeqCst);
    let input = CopyInput::from(CapturedTransferRequest {
        verification: midnight_core::TransferVerification::default(),
        id: OperationId(1),
        destination: root.join("destination"),
        metadata: TransferMetadataPolicy::ContentOnly,
        sources: Arc::new(LeasedCapturedMovePaths {
            paths: CapturedMovePaths(vec![root.join("first"), root.join("second")]),
            _provider: provider,
        }),
    });
    let status = Arc::new(Mutex::new(super::super::initial(&input)));
    let cancel = Cancellation::default();
    let mut preparation = Preparation::new(input, Arc::new(Service(counts.clone())));
    assert!(preparation.attempt(cancel.clone(), status.clone()).is_err());
    assert_eq!(counts.move_opens.load(Ordering::SeqCst), 2);
    assert!(counts.authority_drop_context.lock().unwrap().is_empty());
    assert_eq!(capacity.available_permits(), 0);
    cancel.cancel();
    assert!(preparation.attempt(cancel, status).is_err());
    drop(preparation);

    // These facts were sampled by each inner destructor, not after all owners closed.
    let observed = counts.authority_drop_context.lock().unwrap();
    assert_eq!(
        observed.len(),
        2,
        "one pinned source and one mutation capture"
    );
    assert!(observed.contains(&("source", true)), "{observed:?}");
    assert!(observed.contains(&("capture", true)), "{observed:?}");
    assert_eq!(capacity.available_permits(), 1);
    assert!(
        counts
            .provider
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .is_none()
    );
}
