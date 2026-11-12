use super::*;

#[derive(Default)]
struct Faults {
    operation: &'static str,
    remaining: usize,
    partial: bool,
    published: Option<Vec<u8>>,
}
impl Faults {
    fn fail(&mut self, operation: &'static str) -> io::Result<()> {
        if self.operation == operation && self.remaining != 0 {
            self.remaining -= 1;
            Err(io::Error::new(io::ErrorKind::StorageFull, operation))
        } else {
            Ok(())
        }
    }
}
struct FaultStage {
    data: Cursor<Vec<u8>>,
    faults: Arc<Mutex<Faults>>,
}
impl Read for FaultStage {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let failure = self.faults.lock().unwrap().fail("readback");
        if failure.is_err() {
            // A failed provider call is allowed to disturb its cursor.
            self.data.set_position(self.data.position() + 3);
        }
        failure?;
        self.data.read(bytes)
    }
}
impl Write for FaultStage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut faults = self.faults.lock().unwrap();
        if faults.partial {
            faults.partial = false;
            return self.data.write(&bytes[..bytes.len().min(3)]);
        }
        faults.fail("write")?;
        self.data.write(&bytes[..bytes.len().min(17)])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.faults.lock().unwrap().fail("flush")
    }
}
impl Seek for FaultStage {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.faults.lock().unwrap().fail("seek")?;
        self.data.seek(position)
    }
}
impl StagedTransferFile for FaultStage {
    fn prepare_replacement(
        &mut self,
        _: &OsStr,
        _: Arc<dyn TransferTarget>,
        _: TransferReplacementPolicy,
        _: &Cancellation,
    ) -> Result<(), FsError> {
        self.faults
            .lock()
            .unwrap()
            .fail("replacement")
            .map_err(|error| io_error(Path::new("stage"), error))
    }
    fn sync(&mut self, _: &Cancellation) -> Result<(), FsError> {
        self.faults
            .lock()
            .unwrap()
            .fail("sync")
            .map_err(|e| io_error(Path::new("stage"), e))
    }
    fn len(&self) -> Result<u64, FsError> {
        self.faults
            .lock()
            .unwrap()
            .fail("len")
            .map_err(|e| io_error(Path::new("stage"), e))?;
        Ok(self.data.get_ref().len() as u64)
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        _: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        let mut faults = self.faults.lock().unwrap();
        faults
            .fail("publish")
            .map_err(|e| io_error(Path::new("stage"), e))?;
        faults.published = Some(self.data.get_ref().clone());
        Ok(TransferPublication::Published(name.into()))
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        Ok(())
    }
}

fn fault_copy(
    operation: &'static str,
    remaining: usize,
    partial: bool,
) -> (VerifiedFileCopy, Arc<Mutex<Faults>>, Vec<u8>) {
    let bytes: Vec<u8> = (0..600_007).map(|n| (n % 251) as u8).collect();
    let (mut copy, _) = copy(bytes.clone(), false);
    let faults = Arc::new(Mutex::new(Faults {
        operation,
        remaining,
        partial,
        published: None,
    }));
    copy.stage = Box::new(FaultStage {
        data: Cursor::new(Vec::new()),
        faults: faults.clone(),
    });
    (copy, faults, bytes)
}
fn complete(copy: &mut VerifiedFileCopy, cancel: &Cancellation) {
    for _ in 0..100 {
        if copy.phase() == CopyPhase::ReadyToPublish {
            return;
        }
        copy.step(cancel).unwrap();
    }
    panic!("copy did not complete");
}

#[test]
fn quick_policy_keeps_edge_evidence_through_partial_writes_and_readback_retries() {
    for operation in ["write", "flush", "sync", "seek", "readback", "len"] {
        let (old, faults, bytes) = fault_copy(operation, 1, operation == "write");
        let mut job = VerifiedFileCopy::new_with_verification(
            old.source,
            old.stage,
            midnight_core::TransferVerification::SizeAndSamples,
        )
        .unwrap();
        let cancel = Cancellation::default();
        let mut failures = 0;
        while job.phase() != CopyPhase::ReadyToPublish {
            if job.step(&cancel).is_err() {
                failures += 1;
                assert!(job.can_retry(), "{operation}");
                job.retry(&cancel).unwrap();
            }
        }
        assert_eq!(failures, 1, "{operation}");
        assert_eq!(job.verified_bytes(), 196608);
        job.publish(OsStr::new("target"), &cancel).unwrap();
        assert_eq!(faults.lock().unwrap().published.as_ref(), Some(&bytes));
    }
}

#[test]
fn sampled_readback_retry_restores_middle_and_tail_offsets_not_sample_count() {
    for completed_samples in [1, 2] {
        let (old, faults, bytes) = fault_copy("", 0, false);
        let mut job = VerifiedFileCopy::new_with_verification(
            old.source,
            old.stage,
            midnight_core::TransferVerification::SizeAndSamples,
        )
        .unwrap();
        let cancel = Cancellation::default();
        while job.phase() != CopyPhase::Verifying {
            job.step(&cancel).unwrap();
        }
        for _ in 0..completed_samples {
            job.step(&cancel).unwrap();
        }
        let verified = job.verified_bytes();
        {
            let mut state = faults.lock().unwrap();
            state.operation = "readback";
            state.remaining = 1;
        }
        assert!(job.step(&cancel).is_err());
        assert_eq!(job.verified_bytes(), verified);
        job.retry(&cancel).unwrap();
        complete(&mut job, &cancel);
        assert_eq!(job.verified_bytes(), 196608);
        job.publish(OsStr::new("target"), &cancel).unwrap();
        assert_eq!(faults.lock().unwrap().published.as_ref(), Some(&bytes));
    }
}

#[test]
fn partial_disk_full_write_retries_without_skipping_or_duplicating_bytes() {
    let (mut copy, faults, bytes) = fault_copy("write", 2, true);
    let cancel = Cancellation::default();
    for _ in 0..2 {
        let error = copy.step(&cancel).unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::StorageFull);
        assert_eq!(copy.phase(), CopyPhase::Failed);
        assert_eq!(copy.copied_bytes(), 0);
        assert!(copy.can_retry());
        copy.retry(&cancel).unwrap();
    }
    complete(&mut copy, &cancel);
    copy.publish(OsStr::new("target"), &cancel).unwrap();
    assert_eq!(faults.lock().unwrap().published.as_ref(), Some(&bytes));
    assert_eq!(copy.copied_bytes(), bytes.len() as u64);
    assert!(!copy.can_retry());
}

#[test]
fn partial_write_retry_preserves_already_committed_blocks() {
    let (mut copy, faults, bytes) = fault_copy("", 0, false);
    let cancel = Cancellation::default();
    copy.step(&cancel).unwrap();
    let committed = copy.copied_bytes();
    assert_eq!(committed, 256 * 1024);
    {
        let mut fault = faults.lock().unwrap();
        fault.operation = "write";
        fault.remaining = 1;
        fault.partial = true;
    }
    assert!(copy.step(&cancel).is_err());
    assert_eq!(copy.copied_bytes(), committed);
    copy.retry(&cancel).unwrap();
    complete(&mut copy, &cancel);
    copy.publish(OsStr::new("target"), &cancel).unwrap();
    assert_eq!(faults.lock().unwrap().published.as_ref(), Some(&bytes));
}

#[test]
fn flush_sync_seek_readback_and_length_errors_require_explicit_retry() {
    for operation in ["flush", "sync", "seek", "readback", "len"] {
        let (mut copy, faults, bytes) = fault_copy(operation, 1, false);
        let cancel = Cancellation::default();
        while copy.step(&cancel).is_ok() {}
        assert_eq!(copy.phase(), CopyPhase::Failed, "{operation}");
        assert!(copy.can_retry(), "{operation}");
        copy.retry(&cancel).unwrap();
        complete(&mut copy, &cancel);
        copy.publish(OsStr::new("target"), &cancel).unwrap();
        assert_eq!(
            faults.lock().unwrap().published.as_ref(),
            Some(&bytes),
            "{operation}"
        );
    }
}

#[test]
fn readback_retry_preserves_verified_prefix_after_a_later_read_moves_the_cursor() {
    let (mut copy, faults, bytes) = fault_copy("", 0, false);
    let cancel = Cancellation::default();
    while copy.phase() != CopyPhase::Verifying {
        copy.step(&cancel).unwrap();
    }
    copy.step(&cancel).unwrap();
    let verified = copy.verified_bytes();
    assert_eq!(verified, 256 * 1024);
    {
        let mut fault = faults.lock().unwrap();
        fault.operation = "readback";
        fault.remaining = 1;
    }
    assert!(copy.step(&cancel).is_err());
    assert_eq!(copy.verified_bytes(), verified);
    copy.retry(&cancel).unwrap();
    complete(&mut copy, &cancel);
    copy.publish(OsStr::new("target"), &cancel).unwrap();
    assert_eq!(faults.lock().unwrap().published.as_ref(), Some(&bytes));
    assert_eq!(copy.verified_bytes(), bytes.len() as u64);
}

#[test]
fn publication_error_can_retry_but_success_and_cleanup_cannot() {
    let (mut copy, faults, bytes) = fault_copy("publish", 1, false);
    let cancel = Cancellation::default();
    complete(&mut copy, &cancel);
    assert!(copy.publish(OsStr::new("target"), &cancel).is_err());
    assert!(copy.can_retry());
    copy.retry(&cancel).unwrap();
    copy.publish(OsStr::new("target"), &cancel).unwrap();
    assert_eq!(faults.lock().unwrap().published.as_ref(), Some(&bytes));
    assert!(!copy.can_retry());
    assert!(copy.retry(&cancel).is_err());
    let (mut copy, _, _) = fault_copy("write", 1, false);
    assert!(copy.step(&cancel).is_err());
    copy.cleanup().unwrap();
    assert!(!copy.can_retry());
}

struct FaultSource {
    inner: Source,
    failures: usize,
    supported: bool,
}
impl TransferSource for FaultSource {
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn supports_read_retry(&self) -> bool {
        self.supported
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        self.inner.data.set_position(offset);
        Ok(())
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        if self.failures != 0 && self.inner.data.position() != 0 {
            self.failures -= 1;
            self.inner.data.set_position(self.inner.data.position() + 5);
            return Err(io_error(
                Path::new("source"),
                io::ErrorKind::NotConnected.into(),
            ));
        }
        self.inner.read_chunk(buffer, cancel)
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        unreachable!()
    }
}

#[test]
fn source_read_retry_restores_exact_committed_offset_and_checks_original_revision() {
    for supported in [false, true] {
        let (mut copy, faults, bytes) = fault_copy("", 0, false);
        let changed = Arc::new(Mutex::new(false));
        copy.source = Box::new(FaultSource {
            inner: Source {
                entry: copy.source.entry().clone(),
                data: Cursor::new(bytes.clone()),
                changed: changed.clone(),
            },
            failures: 1,
            supported,
        });
        let cancel = Cancellation::default();
        copy.step(&cancel).unwrap();
        assert!(copy.step(&cancel).is_err());
        assert_eq!(copy.can_retry(), supported);
        if !supported {
            continue;
        }
        copy.retry(&cancel).unwrap();
        complete(&mut copy, &cancel);
        copy.publish(OsStr::new("target"), &cancel).unwrap();
        assert_eq!(faults.lock().unwrap().published.as_ref(), Some(&bytes));
    }
}

#[test]
fn changed_source_cancellation_and_corruption_never_resume_or_publish() {
    let cancel = Cancellation::default();
    let (mut copy, _, bytes) = fault_copy("write", 1, true);
    let changed = Arc::new(Mutex::new(false));
    copy.source = Box::new(Source {
        entry: copy.source.entry().clone(),
        data: Cursor::new(bytes),
        changed: changed.clone(),
    });
    assert!(copy.step(&cancel).is_err());
    *changed.lock().unwrap() = true;
    assert!(copy.retry(&cancel).is_err());
    assert!(!copy.can_retry());
    assert!(copy.publish(OsStr::new("target"), &cancel).is_err());

    let (mut copy, _, _) = fault_copy("write", 1, false);
    assert!(copy.step(&cancel).is_err());
    cancel.cancel();
    assert!(copy.retry(&cancel).is_err());
    assert!(!copy.can_retry());

    let (mut copy, _) = super::copy(b"data".to_vec(), true);
    let cancel = Cancellation::default();
    while copy.step(&cancel).is_ok() {}
    assert!(!copy.can_retry());
    assert!(copy.retry(&cancel).is_err());
}

#[test]
fn failed_checkpoint_seek_stays_retryable_and_preserves_pending_block_and_error_stage() {
    let (mut copy, faults, bytes) = fault_copy("write", 1, true);
    let cancel = Cancellation::default();
    assert!(copy.step(&cancel).is_err());
    assert_eq!(copy.failure_stage(), midnight_core::CopyErrorStage::Writing);
    {
        let mut fault = faults.lock().unwrap();
        fault.operation = "seek";
        fault.remaining = 2;
    }
    for _ in 0..2 {
        assert!(copy.retry(&cancel).is_err());
        assert_eq!(copy.phase(), CopyPhase::Failed);
        assert!(copy.can_retry());
        assert_eq!(copy.copied_bytes(), 0);
    }
    copy.retry(&cancel).unwrap();
    complete(&mut copy, &cancel);
    copy.publish(OsStr::new("target"), &cancel).unwrap();
    assert_eq!(faults.lock().unwrap().published.as_ref(), Some(&bytes));
}

#[test]
fn replacement_preparation_retry_retains_the_consumed_source_and_verified_stage() {
    let (mut copy, faults, bytes) = fault_copy("replacement", 1, false);
    let cancel = Cancellation::default();
    complete(&mut copy, &cancel);
    let target: Arc<dyn TransferTarget> = Arc::new(Target(copy.source.entry().clone()));
    assert!(
        copy.prepare_replacement(
            OsStr::new("target"),
            target.clone(),
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel
        )
        .is_err()
    );
    assert_eq!(
        copy.failure_stage(),
        midnight_core::CopyErrorStage::Preparing
    );
    assert!(copy.can_retry());
    copy.retry(&cancel).unwrap();
    assert_eq!(copy.phase(), CopyPhase::ReadyToPublish);
    copy.prepare_replacement(
        OsStr::new("target"),
        target,
        TransferReplacementPolicy::RecoverableCurrentEntry,
        &cancel,
    )
    .unwrap();
    copy.publish(OsStr::new("different name"), &cancel).unwrap();
    assert_eq!(faults.lock().unwrap().published.as_ref(), Some(&bytes));
    assert_eq!(copy.copied_bytes(), bytes.len() as u64);
}
