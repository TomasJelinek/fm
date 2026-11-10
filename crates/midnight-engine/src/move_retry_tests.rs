use super::*;
use std::{
    ffi::OsStr,
    fs,
    io::Write,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct FailRemoval {
    inner: Arc<dyn TransferMoveCapture>,
    remaining: Arc<AtomicUsize>,
    prepared: Arc<AtomicUsize>,
    receipt: Arc<Mutex<Option<TransferRecovery>>>,
}
impl TransferMoveCapture for FailRemoval {
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
        let outcome = self.inner.capture(cancel)?;
        if let TransferMoveCaptureOutcome::Captured { recovery, .. } = &outcome {
            *self.receipt.lock().unwrap() = Some(recovery.clone());
        }
        Ok(outcome)
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        self.prepared.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FailFinalizer {
            inner: self.inner.prepare_finalization(cancel)?,
            remaining: self.remaining.clone(),
            source: self.entry().item.path.clone(),
            recovery: self.receipt.lock().unwrap().clone().unwrap(),
            outcome: None,
        }))
    }
    fn restore(&self) -> Result<(), FsError> {
        self.inner.restore()
    }
    fn restoration_completed(&self) -> bool {
        self.inner.restoration_completed()
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn midnight_core::TransferMoveStageCleanup>> {
        self.inner.take_stage_cleanup()
    }
}
struct FailFinalizer {
    inner: Box<dyn TransferMoveFinalizer>,
    remaining: Arc<AtomicUsize>,
    source: std::path::PathBuf,
    recovery: TransferRecovery,
    outcome: Option<TransferMoveFinalizeOutcome>,
}
impl TransferMoveFinalizer for FailFinalizer {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn finalize(&mut self, cancel: &Cancellation) -> TransferMoveFinalizeOutcome {
        if let Some(outcome) = &self.outcome {
            return outcome.clone();
        }
        let outcome = if self
            .remaining
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            TransferMoveFinalizeOutcome::Retained {
                source: self.source.clone(),
                recovery: self.recovery.clone(),
                error: error(
                    &self.source,
                    io::ErrorKind::PermissionDenied,
                    "injected sharing violation",
                ),
            }
        } else {
            self.inner.finalize(cancel)
        };
        self.outcome = Some(outcome.clone());
        outcome
    }
}

struct FailProofRead {
    inner: Box<dyn TransferSource>,
    fail: Arc<AtomicBool>,
}
impl TransferSource for FailProofRead {
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.inner.rewind_for_validation(cancel)
    }
    fn read_chunk(&mut self, bytes: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(error(
                &self.entry().item.path,
                io::ErrorKind::PermissionDenied,
                "injected proof read error",
            ));
        }
        self.inner.read_chunk(bytes, cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.inner.next_child(cancel)
    }
}

fn prepared() -> (tempfile::TempDir, Job, Arc<AtomicUsize>, std::path::PathBuf) {
    prepared_with_read_control(Arc::new(AtomicBool::new(false)))
}
fn prepared_with_read_control(
    fail_read: Arc<AtomicBool>,
) -> (tempfile::TempDir, Job, Arc<AtomicUsize>, std::path::PathBuf) {
    let base = std::env::temp_dir().join("midnight-move-retry/_build/tmp");
    fs::create_dir_all(&base).unwrap();
    let temp = tempfile::tempdir_in(base).unwrap();
    let provider = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    fs::write(temp.path().join("seed"), b"seed").unwrap();
    let parent = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let target = parent
        .prepare_target(OsStr::new("seed"), &cancel)
        .unwrap()
        .unwrap();
    let mut stage = parent
        .stage_file_for_target(
            OsStr::new("seed"),
            target,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    stage.write_all(b"new seed").unwrap();
    let TransferPublication::Replaced { recovery, .. } =
        stage.publish_replace(OsStr::new("seed"), &cancel).unwrap()
    else {
        panic!("protected fixture");
    };
    let parent = recovery.path.parent().unwrap().to_owned();
    drop(stage);
    let source = parent.join("first");
    let future = parent.join("second");
    let destination = parent.join("destination");
    fs::write(&source, b"verified first bytes").unwrap();
    fs::write(&future, b"untouched future bytes").unwrap();
    fs::create_dir(&destination).unwrap();
    let request = CopyRequest {
        verification: midnight_core::TransferVerification::default(),
        id: OperationId(1),
        clipboard: FileClipboard {
            paths: vec![source.clone(), future],
            intent: FileClipboardIntent::Move,
        },
        destination,
    };
    let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
    let mut job = prepare(request, &provider, cancel, status).unwrap();
    let first = job.roots.pop_front().unwrap();
    job.roots.push_front(Box::new(FailProofRead {
        inner: first,
        fail: fail_read,
    }));
    let work = job.move_work.as_mut().unwrap();
    let prepared = Arc::new(AtomicUsize::new(0));
    work.captures[0] = Arc::new(FailRemoval {
        inner: work.captures[0].clone(),
        remaining: Arc::new(AtomicUsize::new(1)),
        prepared: prepared.clone(),
        receipt: Arc::new(Mutex::new(None)),
    });
    for _ in 0..100 {
        if job.move_work.as_ref().unwrap().has_pending() {
            return (temp, job, prepared, source);
        }
        assert!(matches!(job.step(), Step::Continue));
    }
    panic!("copy did not publish");
}
fn fail(work: &mut MoveWork, job: &mut Job) {
    for _ in 0..100 {
        if work.advance(job).is_err() {
            return;
        }
    }
    panic!("injected removal failure was not reached");
}

#[test]
fn retry_removal_rebuilds_finalizer_and_rehashes_both_retained_objects() {
    let (_temp, mut job, prepared, source) = prepared();
    let mut work = job.move_work.take().unwrap();
    fail(&mut work, &mut job);
    assert_eq!(work.failure_stage(), CopyErrorStage::RemovingSource);
    assert!(work.can_retry());
    assert!(work.can_skip_failure());
    let validated = job.progress.validation_bytes;
    let copied = job.progress.copied_bytes;
    work.retry(&mut job).unwrap();
    for _ in 0..100 {
        work.advance(&mut job).unwrap();
        if !work.has_pending() {
            break;
        }
    }
    assert_eq!(prepared.load(Ordering::SeqCst), 2);
    assert_eq!(
        job.progress.validation_bytes - validated,
        2 * b"verified first bytes".len() as u64
    );
    assert_eq!(job.progress.copied_bytes, copied);
    assert_eq!(job.progress.source_files_removed, 1);
    assert!(!source.exists());
    assert!(source.parent().unwrap().join("second").exists());
    assert_eq!(work.captures.len(), 1);
    assert!(
        work.stages.is_empty(),
        "future sources have no eager copy stages"
    );
    assert!(!work.can_retry());
    assert!(!work.can_skip_failure());
    assert!(work.cleanup(&mut job).is_empty());
}

#[test]
fn skip_failed_removal_restores_only_this_source_and_retains_future_work() {
    let (_temp, mut job, _, source) = prepared();
    let mut work = job.move_work.take().unwrap();
    fail(&mut work, &mut job);
    work.skip_failed(&mut job).unwrap();
    assert_eq!(fs::read(&source).unwrap(), b"verified first bytes");
    assert_eq!(
        fs::read(job.progress.destination_directory.join("first")).unwrap(),
        b"verified first bytes"
    );
    assert_eq!(job.progress.source_files_removed, 0);
    assert_eq!(work.captures.len(), 1);
    assert!(
        work.stages.is_empty(),
        "future sources have no eager copy stages"
    );
    assert!(!work.has_pending());
    assert!(!work.can_retry());
    assert!(work.cleanup(&mut job).is_empty());
}

#[test]
fn failed_restore_keeps_new_source_winner_and_reports_the_retained_original() {
    let (_temp, mut job, _, source) = prepared();
    let mut work = job.move_work.take().unwrap();
    fail(&mut work, &mut job);
    let retained = work.active.as_ref().unwrap().1.path.clone();
    fs::write(&source, b"new source winner").unwrap();
    assert!(work.skip_failed(&mut job).is_err());
    assert!(!work.can_retry());
    assert!(!work.can_skip_failure());
    assert!(!work.cleanup(&mut job).is_empty());
    assert_eq!(fs::read(&source).unwrap(), b"new source winner");
    assert_eq!(fs::read(&retained).unwrap(), b"verified first bytes");
    assert_eq!(job.progress.source_files_removed, 0);
    assert_eq!(job.progress.source_residual_count, 1);
    assert_eq!(job.progress.source_residuals[0].temporary.path, retained);
}

#[test]
fn move_proof_read_error_has_a_different_skip_all_stage_from_source_removal() {
    let fail_read = Arc::new(AtomicBool::new(false));
    let (_temp, mut job, _, source) = prepared_with_read_control(fail_read.clone());
    let mut work = job.move_work.take().unwrap();
    fail_read.store(true, Ordering::SeqCst);
    fail(&mut work, &mut job);
    assert_eq!(work.failure_stage(), CopyErrorStage::Verifying);
    assert!(!work.can_retry());
    assert!(work.can_skip_failure());
    work.skip_failed(&mut job).unwrap();
    assert_eq!(fs::read(source).unwrap(), b"verified first bytes");
    assert_eq!(job.progress.source_files_removed, 0);
    assert!(work.cleanup(&mut job).is_empty());
}
