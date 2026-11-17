use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct Counts {
    live: AtomicUsize,
    peak: AtomicUsize,
    opened: AtomicUsize,
    writes: AtomicUsize,
    unknown: AtomicBool,
    fail_pin: AtomicBool,
    deny_directory_reads: AtomicBool,
}
struct Service {
    native: midnight_platform::LocalFileSystem,
    counts: Arc<Counts>,
    opaque: bool,
}
// This factory owns native synchronous delegates and pure host counters. Its
// Unknown pin is metadata only; geometry does not determine retirement proof.
impl midnight_core::AuditedSynchronousTransferService for Service {}
impl Service {
    fn source(&self, source: Box<dyn TransferSource>) -> Box<dyn TransferSource> {
        let live = self.counts.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.counts.peak.fetch_max(live, Ordering::SeqCst);
        self.counts.opened.fetch_add(1, Ordering::SeqCst);
        Box::new(Counted {
            source: SharedSource::new(source),
            _live: Live(self.counts.clone()),
            opaque: self.opaque,
        })
    }
}
impl TransferService for Service {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Ok(self.source(self.native.prepare_transfer_source(path, cancel)?))
    }
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        let prepared = self.native.prepare_move_source(path, cancel)?;
        Ok(PreparedTransferMove {
            source: self.source(prepared.source),
            capture: prepared.capture,
        })
    }
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        if self.counts.deny_directory_reads.load(Ordering::SeqCst) {
            return Err(error(
                path,
                io::ErrorKind::PermissionDenied,
                "directory enumeration is denied",
            ));
        }
        self.native.prepare_checksum_directory(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.counts.writes.fetch_add(1, Ordering::SeqCst);
        self.native.prepare_transfer_destination(path, cancel)
    }
}
struct Counted {
    // Close the real native owner before recording that it is gone.
    source: SharedSource,
    _live: Live,
    opaque: bool,
}
struct Live(Arc<Counts>);
impl Drop for Live {
    fn drop(&mut self) {
        self.0.live.fetch_sub(1, Ordering::SeqCst);
    }
}
impl TransferSource for Counted {
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
        if self._live.0.fail_pin.swap(false, Ordering::SeqCst) {
            return Err(error(
                &self.entry().item.path,
                io::ErrorKind::NotConnected,
                "temporary pin qualification failure",
            ));
        }
        if self.opaque {
            Ok(None)
        } else if self._live.0.unknown.load(Ordering::SeqCst) {
            Ok(Some(Arc::new(Unknown)))
        } else {
            self.source.native_namespace_scope(cancel)
        }
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
        self.source.read_chunk(buffer, cancel)
    }
    fn supports_read_retry(&self) -> bool {
        self.source.supports_read_retry()
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.source.retry_read_from(offset, cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.source.next_child(cancel)
    }
}
struct Unknown;
impl TransferNamespacePin for Unknown {
    fn refresh(&self, _: &Cancellation) -> Result<TransferNamespaceProbe, FsError> {
        Ok(TransferNamespaceProbe::Unknown)
    }
}
fn fixture(opaque: bool) -> (tempfile::TempDir, RetainedService, Arc<Counts>) {
    let temp = tempfile::tempdir().unwrap();
    let counts = Arc::new(Counts::default());
    let service = RetainedService::new(Arc::new(Service {
        native: midnight_platform::LocalFileSystem::new(None),
        counts: counts.clone(),
        opaque,
    }));
    (temp, service, counts)
}
fn known(source: &dyn TransferSource, cancel: &Cancellation) {
    assert!(
        matches!(source.native_namespace_scope(cancel).unwrap().unwrap().refresh(cancel).unwrap(), TransferNamespaceProbe::Known(scopes) if !scopes.is_empty()),
        "this native qualification fixture requires a supported local lookup domain"
    );
}

#[test]
fn planned_native_roots_and_discovery_release_each_real_source_before_the_next() {
    let (temp, service, counts) = fixture(false);
    let cancel = Cancellation::default();
    let mut roots = Vec::new();
    for index in 0..256 {
        let path = temp.path().join(index.to_string());
        std::fs::write(&path, b"payload").unwrap();
        let source = service.prepare_transfer_source(&path, &cancel).unwrap();
        known(source.as_ref(), &cancel);
        roots.push(source);
        assert_eq!(counts.live.load(Ordering::SeqCst), 0);
    }
    let mut seen = 0;
    for source in &roots {
        source.discover(&cancel, &mut |_| seen += 1).unwrap();
        assert_eq!(counts.live.load(Ordering::SeqCst), 0);
    }
    assert_eq!(seen, 256);
    assert_eq!(counts.peak.load(Ordering::SeqCst), 1);
}

#[test]
fn active_retry_keeps_the_first_reopened_native_owner_and_offset() {
    let (temp, service, counts) = fixture(false);
    let path = temp.path().join("source");
    std::fs::write(&path, b"abcdef").unwrap();
    let cancel = Cancellation::default();
    let mut source = service.prepare_transfer_source(&path, &cancel).unwrap();
    known(source.as_ref(), &cancel);
    let mut first = [0; 2];
    assert_eq!(source.read_chunk(&mut first, &cancel).unwrap(), 2);
    assert_eq!(&first, b"ab");
    source.retry_read_from(2, &cancel).unwrap();
    assert_eq!(source.read_chunk(&mut first, &cancel).unwrap(), 2);
    assert_eq!(&first, b"cd");
    assert_eq!(counts.opened.load(Ordering::SeqCst), 2);
    assert_eq!(counts.live.load(Ordering::SeqCst), 1);
}

#[test]
fn replaced_original_binding_is_refused_before_opening_an_active_source() {
    let (temp, service, counts) = fixture(false);
    let path = temp.path().join("source");
    std::fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    let mut source = service.prepare_transfer_source(&path, &cancel).unwrap();
    known(source.as_ref(), &cancel);
    std::fs::rename(&path, temp.path().join("old")).unwrap();
    std::fs::write(&path, b"replacement").unwrap();
    assert!(source.read_chunk(&mut [0; 1], &cancel).is_err());
    assert_eq!(counts.opened.load(Ordering::SeqCst), 1);
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
}

#[test]
fn opaque_sources_keep_the_original_ready_owner() {
    for no_capability in [true, false] {
        let (temp, service, counts) = fixture(no_capability);
        counts.unknown.store(!no_capability, Ordering::SeqCst);
        let path = temp.path().join("source");
        std::fs::write(&path, b"original").unwrap();
        let mut source = service
            .prepare_transfer_source(&path, &Cancellation::default())
            .unwrap();
        let pin = source
            .native_namespace_scope(&Cancellation::default())
            .unwrap();
        if no_capability {
            assert!(pin.is_none());
        } else {
            assert!(matches!(
                pin.unwrap().refresh(&Cancellation::default()).unwrap(),
                TransferNamespaceProbe::Unknown
            ));
        }
        assert_eq!(counts.live.load(Ordering::SeqCst), 1);
        source
            .read_chunk(&mut [0; 1], &Cancellation::default())
            .unwrap();
        assert_eq!(counts.opened.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn failed_pin_qualification_retains_original_owner_for_retry() {
    let (temp, service, counts) = fixture(false);
    let path = temp.path().join("source");
    std::fs::write(&path, b"original").unwrap();
    counts.fail_pin.store(true, Ordering::SeqCst);
    assert!(
        service
            .prepare_transfer_source(&path, &Cancellation::default())
            .is_err()
    );
    assert_eq!(counts.live.load(Ordering::SeqCst), 1);
    let mut source = service
        .prepare_transfer_source(&path, &Cancellation::default())
        .unwrap();
    assert_eq!(
        source
            .read_chunk(&mut [0; 1], &Cancellation::default())
            .unwrap(),
        1
    );
    assert_eq!(counts.opened.load(Ordering::SeqCst), 1);
}

#[test]
fn endpoint_planning_opens_only_read_only_destination_and_uses_exact_children() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::write(&source, b"data").unwrap();
    std::fs::create_dir(&destination).unwrap();
    let counts = Arc::new(Counts::default());
    let input = CopyInput::from(CopyRequest {
        id: OperationId(1),
        clipboard: FileClipboard {
            paths: vec![source],
            intent: FileClipboardIntent::Copy,
        },
        destination,
        verification: TransferVerification::default(),
        start_policy: TransferStartPolicy::Immediate,
    });
    let status = Arc::new(Mutex::new(super::super::initial(&input)));
    let mut preparation = Preparation::qualified(
        input,
        crate::transfer_composition::QualifiedTransferInstallation::audited(Arc::new(Service {
            native: midnight_platform::LocalFileSystem::new(None),
            counts: counts.clone(),
            opaque: false,
        })),
    );
    let endpoints = preparation
        .pin_endpoints(&Cancellation::default(), &status)
        .unwrap();
    assert_eq!(endpoints.len(), 2);
    assert_eq!(endpoints[0].0, TransferNamespaceAccess::Read);
    assert_eq!(endpoints[1].0, TransferNamespaceAccess::Write);
    let TransferNamespaceProbe::Known(target) = endpoints[1]
        .1
        .as_ref()
        .unwrap()
        .refresh(&Cancellation::default())
        .unwrap()
    else {
        panic!("qualified native target required")
    };
    assert_eq!(target.len(), 1);
    assert!(target[0].object.is_none());
    let TransferNamespaceProbe::Known(source_scope) = endpoints[0]
        .1
        .as_ref()
        .unwrap()
        .refresh(&Cancellation::default())
        .unwrap()
    else {
        panic!("qualified native source required")
    };
    assert_eq!(target[0].lookup_key, source_scope[0].lookup_key);
    assert_eq!(counts.writes.load(Ordering::SeqCst), 0);
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
    assert_eq!(status.lock().unwrap().requested_roots, 1);
}

#[test]
fn metadata_only_move_can_plan_an_opaque_destination_without_directory_read_rights() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::write(&source, b"payload").unwrap();
    std::fs::create_dir(&destination).unwrap();
    let input = CopyInput::move_clipboard(ClipboardMoveRequest {
        id: OperationId(1),
        clipboard: Arc::new(FileClipboard {
            paths: vec![source],
            intent: FileClipboardIntent::Move,
        }),
        destination,
        verification: TransferVerification::default(),
        start_policy: TransferStartPolicy::Immediate,
    });
    let status = Arc::new(Mutex::new(super::super::initial(&input)));
    let counts = Arc::new(Counts::default());
    counts.deny_directory_reads.store(true, Ordering::SeqCst);
    let mut preparation = Preparation::qualified(
        input,
        crate::transfer_composition::QualifiedTransferInstallation::audited(Arc::new(Service {
            native: midnight_platform::LocalFileSystem::new(None),
            counts: counts.clone(),
            opaque: false,
        })),
    );
    let endpoints = preparation
        .pin_endpoints(&Cancellation::default(), &status)
        .unwrap();
    assert_eq!(endpoints.len(), 2);
    assert_eq!(endpoints[0].0, TransferNamespaceAccess::Write);
    assert_eq!(endpoints[1].0, TransferNamespaceAccess::Write);
    assert!(endpoints[1].1.is_none());
    assert_eq!(counts.writes.load(Ordering::SeqCst), 0);
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
}

#[test]
fn move_metadata_plans_drop_pairs_and_activate_only_the_used_pair() {
    let (temp, mut service, counts) = fixture(false);
    let cancel = Cancellation::default();
    let mut roots = Vec::new();
    for index in 0..8 {
        let path = temp.path().join(index.to_string());
        std::fs::write(&path, b"payload").unwrap();
        let root = service.prepare_move_source(&path, &cancel).unwrap();
        known(root.source.as_ref(), &cancel);
        root.capture.revalidate(&cancel).unwrap();
        roots.push(root);
        assert_eq!(counts.live.load(Ordering::SeqCst), 0);
    }
    let destination = temp.path().join("destination");
    std::fs::create_dir(&destination).unwrap();
    let native = midnight_platform::LocalFileSystem::new(None);
    let target = native
        .prepare_move_destination(&destination, &cancel)
        .unwrap();
    assert!(matches!(
        roots[0]
            .capture
            .try_native_move(target.as_ref(), OsStr::new("moved"), None, &cancel)
            .unwrap(),
        TransferNativeMoveOutcome::Moved { .. }
    ));
    assert_eq!(
        std::fs::read(destination.join("moved")).unwrap(),
        b"payload"
    );
    assert!(!temp.path().join("0").exists());
    assert_eq!(counts.opened.load(Ordering::SeqCst), 9);
    assert_eq!(counts.live.load(Ordering::SeqCst), 1);
    // Repeated native completion uses the retained capture's committed outcome.
    assert!(matches!(
        roots[0]
            .capture
            .try_native_move(target.as_ref(), OsStr::new("moved"), None, &cancel)
            .unwrap(),
        TransferNativeMoveOutcome::Moved { .. }
    ));
    assert_eq!(counts.opened.load(Ordering::SeqCst), 9);
    service.release();
    drop(roots);
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
}

#[test]
fn same_inode_content_revision_change_is_refused_before_activation() {
    let (temp, service, counts) = fixture(false);
    let path = temp.path().join("source");
    std::fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    let mut source = service.prepare_transfer_source(&path, &cancel).unwrap();
    known(source.as_ref(), &cancel);
    // Preserve the inode but change a revision component with no timing assumption.
    std::fs::write(&path, b"longer replacement contents").unwrap();
    assert!(source.read_chunk(&mut [0; 1], &cancel).is_err());
    assert_eq!(counts.opened.load(Ordering::SeqCst), 1);
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
}

#[derive(Debug)]
struct Captured(Vec<PathBuf>);
impl CapturedTransferSources for Captured {
    fn prepares_audited_native_children(&self) -> bool {
        true
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        // The capture stores only paths; all returned native children are made
        // by the independently supplied service in prepare below.
        drop(Arc::into_inner(self));
        Ok(())
    }
    fn intent(&self) -> TransferIntent {
        TransferIntent::Copy
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
            .map(|path| service.prepare_transfer_source(path, cancel))
            .collect::<Result<_, _>>()
            .map(PreparedCapturedTransfer::Copy)
    }
}
#[test]
fn captured_native_roots_keep_deferred_owners_and_original_plan_on_retry() {
    let temp = tempfile::tempdir().unwrap();
    let roots: Vec<_> = (0..128)
        .map(|index| temp.path().join(index.to_string()))
        .collect();
    for path in &roots {
        std::fs::write(path, b"data").unwrap();
    }
    let destination = temp.path().join("destination");
    std::fs::create_dir(&destination).unwrap();
    let input = CopyInput::from(CapturedTransferRequest {
        id: OperationId(1),
        sources: Arc::new(Captured(roots.clone())),
        destination,
        metadata: TransferMetadataPolicy::ContentOnly,
        verification: TransferVerification::default(),
        start_policy: TransferStartPolicy::Immediate,
    });
    let status = Arc::new(Mutex::new(super::super::initial(&input)));
    let counts = Arc::new(Counts::default());
    let mut preparation = Preparation::qualified(
        input,
        crate::transfer_composition::QualifiedTransferInstallation::audited(Arc::new(Service {
            native: midnight_platform::LocalFileSystem::new(None),
            counts: counts.clone(),
            opaque: false,
        })),
    );
    assert_eq!(
        preparation
            .pin_endpoints(&Cancellation::default(), &status)
            .unwrap()
            .len(),
        256
    );
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
    assert_eq!(counts.peak.load(Ordering::SeqCst), 1);
    std::fs::rename(&roots[0], temp.path().join("old")).unwrap();
    std::fs::write(&roots[0], b"data").unwrap();
    assert!(
        preparation
            .pin_endpoints(&Cancellation::default(), &status)
            .is_err()
    );
    assert_eq!(counts.opened.load(Ordering::SeqCst), 128);
    assert_eq!(counts.writes.load(Ordering::SeqCst), 0);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "explicit native scale Build lease; 100000 real files and metadata probes"]
fn hundred_thousand_native_roots_keep_actual_descriptor_peak_bounded() {
    let (temp, service, counts) = fixture(false);
    let baseline = std::fs::read_dir("/proc/self/fd").unwrap().count();
    let cancel = Cancellation::default();
    let mut roots = Vec::new();
    for index in 0..100_000 {
        let path = temp.path().join(index.to_string());
        std::fs::write(&path, []).unwrap();
        let source = service.prepare_transfer_source(&path, &cancel).unwrap();
        known(source.as_ref(), &cancel);
        roots.push(source);
        if index % 1000 == 0 {
            assert!(std::fs::read_dir("/proc/self/fd").unwrap().count() <= baseline + 16);
        }
    }
    assert_eq!(roots.len(), 100_000);
    assert_eq!(counts.peak.load(Ordering::SeqCst), 1);
    assert_eq!(counts.live.load(Ordering::SeqCst), 0);
    assert!(std::fs::read_dir("/proc/self/fd").unwrap().count() <= baseline + 16);
}
