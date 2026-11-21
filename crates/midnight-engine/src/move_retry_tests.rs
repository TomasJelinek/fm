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
    let prepared = Arc::new(AtomicUsize::new(0));
    let counted = prepared.clone();
    let (temp, job, source) = prepared_with(fail_read, move |inner| {
        Arc::new(FailRemoval {
            inner,
            remaining: Arc::new(AtomicUsize::new(1)),
            prepared: counted,
            receipt: Arc::new(Mutex::new(None)),
        })
    });
    (temp, job, prepared, source)
}
/// A Move of `first` and `second` whose first file is published and its proof pending;
/// `wrap` replaces the first file's capture.
fn prepared_with(
    fail_read: Arc<AtomicBool>,
    wrap: impl FnOnce(Arc<dyn TransferMoveCapture>) -> Arc<dyn TransferMoveCapture>,
) -> (tempfile::TempDir, Job, std::path::PathBuf) {
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
        start_policy: midnight_core::TransferStartPolicy::Immediate,
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
    work.captures[0] = wrap(work.captures[0].clone());
    let mut discovery = OperationDiscovery::scanning(DiscoveryScope::Recursive);
    let mut budget = work_budget::Budget::discovery();
    for source in &job.roots {
        discovery.observe(&source.entry().item);
        budget.observe(&source.entry().item, job.verification);
    }
    discovery.finish();
    job.progress.discovery = discovery;
    job.work_budget = budget;
    for _ in 0..100 {
        if job.move_work.as_ref().unwrap().has_pending() {
            return (temp, job, source);
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
    assert_eq!(work.active_proof_credit(job.verification), None);
    work.retry(&mut job).unwrap();
    let future = b"untouched future bytes".len() as u64;
    let repeated = 2 * b"verified first bytes".len() as u64;
    assert_eq!(work.active_proof_credit(job.verification), Some(repeated));
    let remaining = job
        .work_budget
        .remaining(&job.progress, work.active_proof_credit(job.verification))
        .unwrap();
    assert_eq!(remaining.write_bytes, future);
    assert_eq!(remaining.verification_bytes, 5 * future + repeated);
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
    assert!(work.skip_failed(&mut job).unwrap().is_none());
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
    assert!(work.skip_failed(&mut job).unwrap().is_none());
    assert_eq!(fs::read(source).unwrap(), b"verified first bytes");
    assert_eq!(job.progress.source_files_removed, 0);
    assert!(work.cleanup(&mut job).is_empty());
}

/// Removes the verified source, then refuses to remove its empty private stage.
struct RefusedStageCleanup(Arc<dyn TransferMoveCapture>);
impl TransferMoveCapture for RefusedStageCleanup {
    fn entry(&self) -> &TransferEntry {
        self.0.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.revalidate(cancel)
    }
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.preflight_finalization(cancel)
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.0.capture(cancel)
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        self.0.prepare_finalization(cancel)
    }
    fn restore(&self) -> Result<(), FsError> {
        self.0.restore()
    }
    fn restoration_completed(&self) -> bool {
        self.0.restoration_completed()
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn midnight_core::TransferMoveStageCleanup>> {
        let cleanup = self.0.take_stage_cleanup()?;
        Some(Box::new(RefusedCleanup(cleanup)))
    }
}
struct RefusedCleanup(Box<dyn midnight_core::TransferMoveStageCleanup>);
impl midnight_core::TransferMoveStageCleanup for RefusedCleanup {
    fn path(&self) -> &Path {
        self.0.path()
    }
    fn cleanup(self: Box<Self>) -> Result<(), FsError> {
        Err(error(
            self.0.path(),
            io::ErrorKind::PermissionDenied,
            "injected stage cleanup refusal",
        ))
    }
}

/// Removes the verified source, then reports that its removal may not be durable.
struct UnconfirmedRemoval(Arc<dyn TransferMoveCapture>);
impl TransferMoveCapture for UnconfirmedRemoval {
    fn entry(&self) -> &TransferEntry {
        self.0.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.revalidate(cancel)
    }
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.preflight_finalization(cancel)
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.0.capture(cancel)
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        Ok(Box::new(UnconfirmedFinalizer(
            self.0.prepare_finalization(cancel)?,
        )))
    }
    fn restore(&self) -> Result<(), FsError> {
        self.0.restore()
    }
    fn restoration_completed(&self) -> bool {
        self.0.restoration_completed()
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn midnight_core::TransferMoveStageCleanup>> {
        self.0.take_stage_cleanup()
    }
}
struct UnconfirmedFinalizer(Box<dyn TransferMoveFinalizer>);
impl TransferMoveFinalizer for UnconfirmedFinalizer {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.revalidate(cancel)
    }
    fn finalize(&mut self, cancel: &Cancellation) -> TransferMoveFinalizeOutcome {
        match self.0.finalize(cancel) {
            TransferMoveFinalizeOutcome::Removed {
                source, recovery, ..
            } => TransferMoveFinalizeOutcome::Removed {
                source,
                recovery,
                state: TransferSourceRemoval::Unlinked,
                durability_confirmed: false,
            },
            retained => retained,
        }
    }
}

fn refused_stage_cleanup(capture: Arc<dyn TransferMoveCapture>) -> Arc<dyn TransferMoveCapture> {
    Arc::new(RefusedStageCleanup(capture))
}
fn unconfirmed_removal(capture: Arc<dyn TransferMoveCapture>) -> Arc<dyn TransferMoveCapture> {
    Arc::new(UnconfirmedRemoval(capture))
}

/// The next question of `job`, failing if it ends first.
fn question(job: &mut Job) -> Arc<CopyFailure> {
    for _ in 0..200 {
        match job.step() {
            Step::Continue => {}
            Step::Failure(failure) => return failure,
            Step::Done => panic!("the job ended without a question"),
            Step::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
        }
    }
    panic!("no question within bounded steps");
}

#[test]
fn an_uncertain_end_of_a_verified_removal_asks_skip_only_and_skip_moves_the_next_file() {
    for (wrap, stage) in [
        (refused_stage_cleanup as fn(_) -> _, CopyErrorStage::Cleanup),
        (unconfirmed_removal, CopyErrorStage::RemovingSource),
    ] {
        let (_temp, mut job, source) = prepared_with(Arc::new(AtomicBool::new(false)), wrap);
        let failure = question(&mut job);
        assert_eq!(failure.stage, stage, "{failure:?}");
        assert!(!failure.can_retry && failure.can_skip, "{failure:?}");
        assert_eq!(failure.source, Some(source.clone()));
        assert_eq!(
            failure.destination,
            job.progress.destination_directory.join("first")
        );
        // The verified source is gone and its copy in place, recorded as it happened.
        assert!(!source.exists());
        assert_eq!(job.progress.source_files_removed, 1);
        assert_eq!(job.progress.failed, 1);
        for (choice, accepted) in [(CopyChoice::Retry, false), (CopyChoice::Skip, true)] {
            assert_eq!(
                job.accept_decision(CopyDecision {
                    id: failure.id,
                    version: failure.version,
                    choice,
                }),
                accepted
            );
        }
        for _ in 0..200 {
            match job.step() {
                Step::Continue => {}
                Step::Done => break,
                Step::Failure(failure) => panic!("asked again: {failure:?}"),
                Step::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
            }
        }
        let destination = job.progress.destination_directory.clone();
        let done = job.finish();
        assert_eq!(
            done.state,
            CopyOperationState::CompletedWithErrors,
            "{done:?}"
        );
        assert_eq!((done.failed, done.skipped), (1, 0), "{done:?}");
        // The second file moves too (by its own rename on this one volume).
        assert_eq!(
            done.source_files_removed + done.native_entries_moved,
            2,
            "{done:?}"
        );
        assert_eq!(
            fs::read(destination.join("first")).unwrap(),
            b"verified first bytes"
        );
        assert_eq!(
            fs::read(destination.join("second")).unwrap(),
            b"untouched future bytes"
        );
        assert!(!source.with_file_name("second").exists());
    }
}

#[test]
fn a_skipped_removal_whose_stage_remains_after_restoration_asks_only_whether_to_go_on() {
    let (_temp, mut job, source) = prepared_with(Arc::new(AtomicBool::new(false)), |inner| {
        refused_stage_cleanup(Arc::new(FailRemoval {
            inner,
            remaining: Arc::new(AtomicUsize::new(1)),
            prepared: Arc::new(AtomicUsize::new(0)),
            receipt: Arc::new(Mutex::new(None)),
        }))
    });
    let failure = question(&mut job);
    assert_eq!(failure.stage, CopyErrorStage::RemovingSource, "{failure:?}");
    assert!(job.accept_decision(CopyDecision {
        id: failure.id,
        version: failure.version,
        choice: CopyChoice::Skip,
    }));
    // The source is back under its name, but its private stage cannot be removed.
    let cleanup = question(&mut job);
    assert_eq!(cleanup.stage, CopyErrorStage::Cleanup, "{cleanup:?}");
    assert!(!cleanup.can_retry && cleanup.can_skip, "{cleanup:?}");
    assert_eq!(cleanup.source, Some(source.clone()));
    assert_eq!(fs::read(&source).unwrap(), b"verified first bytes");
    assert!(job.accept_decision(CopyDecision {
        id: cleanup.id,
        version: cleanup.version,
        choice: CopyChoice::Skip,
    }));
    for _ in 0..200 {
        match job.step() {
            Step::Continue => {}
            Step::Done => break,
            Step::Failure(failure) => panic!("asked again: {failure:?}"),
            Step::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
        }
    }
    let destination = job.progress.destination_directory.clone();
    let done = job.finish();
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!(done.skipped, 1, "{done:?}");
    // The skipped file stays, next to its copy; the second file moves.
    assert_eq!(fs::read(&source).unwrap(), b"verified first bytes");
    assert_eq!(
        fs::read(destination.join("first")).unwrap(),
        b"verified first bytes"
    );
    assert_eq!(
        fs::read(destination.join("second")).unwrap(),
        b"untouched future bytes"
    );
    assert!(!source.with_file_name("second").exists());
}
