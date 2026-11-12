//! The real queue must choose native namespace moves before any content authority.
use super::*;
use std::ffi::OsStr;

#[derive(Clone, Copy)]
enum NativeResult {
    Moved,
    CancelAfterCommit,
    PermissionDenied,
    UnsupportedIo,
    CrossDevice,
    Unconfirmed,
    RestoredFailure,
    CommittedFailure,
    BlockedCommit,
    SameEntry,
    Replace,
}
struct Counts {
    native: AtomicUsize,
    proof: AtomicUsize,
    reads: AtomicUsize,
    lists: AtomicUsize,
    stages: AtomicUsize,
    result: NativeResult,
    kind: EntryKind,
    entered: Notify,
    released: Arc<AtomicBool>,
}
impl Counts {
    fn new(kind: EntryKind, result: NativeResult) -> Arc<Self> {
        Arc::new(Self {
            native: AtomicUsize::new(0),
            proof: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            lists: AtomicUsize::new(0),
            stages: AtomicUsize::new(0),
            result,
            kind,
            entered: Notify::new(),
            released: Arc::new(AtomicBool::new(false)),
        })
    }
}
fn unsupported(path: &Path) -> FsError {
    error(
        path,
        io::ErrorKind::Unsupported,
        "Content authority must remain unused",
    )
}
struct Service(Arc<Counts>);
impl TransferService for Service {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Err(unsupported(path))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(Directory {
            path: path.into(),
            counts: self.0.clone(),
        }))
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.prepare_transfer_destination(path, cancel)
    }
    fn prepare_move_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        let entry = TransferEntry {
            identity: TransferIdentity {
                device: 1,
                object: [1; 16],
            },
            item: MutationItem {
                path: path.into(),
                kind: self.0.kind,
                metadata: EntryMetadata {
                    size: (self.0.kind == EntryKind::File).then_some(1_000_000),
                    ..EntryMetadata::default()
                },
            },
        };
        Ok(PreparedTransferMove {
            source: Box::new(Source {
                entry: entry.clone(),
                counts: self.0.clone(),
            }),
            capture: Arc::new(Capture {
                entry,
                counts: self.0.clone(),
            }),
        })
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
        self.counts.reads.fetch_add(1, Ordering::SeqCst);
        Err(unsupported(&self.entry.item.path))
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.counts.lists.fetch_add(1, Ordering::SeqCst);
        Err(unsupported(&self.entry.item.path))
    }
    fn next_move_child(
        &mut self,
        _: &Cancellation,
    ) -> Result<Option<PreparedTransferMove>, FsError> {
        self.counts.lists.fetch_add(1, Ordering::SeqCst);
        Err(unsupported(&self.entry.item.path))
    }
}
struct Capture {
    entry: TransferEntry,
    counts: Arc<Counts>,
}
impl TransferMoveCapture for Capture {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn preflight_finalization(&self, _: &Cancellation) -> Result<(), FsError> {
        self.counts.proof.fetch_add(1, Ordering::SeqCst);
        Err(unsupported(&self.entry.item.path))
    }
    fn capture(&self, _: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        panic!("native move must not use verified source deletion")
    }
    fn try_native_move(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        if matches!(self.counts.result, NativeResult::Replace) {
            let (target, policy) =
                replace.expect("replacement requires a retained consented target");
            assert_eq!(policy, TransferReplacementPolicy::RecoverableCurrentEntry);
            assert_eq!(target.entry().identity.object, [3; 16]);
        } else {
            assert!(replace.is_none());
        }
        self.counts.native.fetch_add(1, Ordering::SeqCst);
        match self.counts.result {
            NativeResult::PermissionDenied => Err(error(
                &self.entry.item.path,
                io::ErrorKind::PermissionDenied,
                "Native rename denied",
            )),
            NativeResult::CrossDevice => Ok(TransferNativeMoveOutcome::CrossDevice),
            NativeResult::SameEntry => Ok(TransferNativeMoveOutcome::SameEntry),
            NativeResult::UnsupportedIo => Err(unsupported(&self.entry.item.path)),
            NativeResult::RestoredFailure | NativeResult::CommittedFailure => {
                Ok(TransferNativeMoveOutcome::RecoveryRequired {
                    destination: matches!(self.counts.result, NativeResult::CommittedFailure)
                        .then(|| destination.path().join(name)),
                    recovery: vec![],
                    error: error(
                        &self.entry.item.path,
                        io::ErrorKind::Other,
                        "Native namespace changed before failure",
                    ),
                })
            }
            NativeResult::Moved
            | NativeResult::CancelAfterCommit
            | NativeResult::Unconfirmed
            | NativeResult::BlockedCommit
            | NativeResult::Replace => {
                if matches!(self.counts.result, NativeResult::CancelAfterCommit) {
                    cancel.cancel();
                }
                if matches!(self.counts.result, NativeResult::BlockedCommit) {
                    self.counts.entered.notify_one();
                    while !self.counts.released.load(Ordering::Acquire) {
                        std::thread::park_timeout(Duration::from_millis(5));
                    }
                }
                Ok(TransferNativeMoveOutcome::Moved {
                    destination: destination.path().join(name),
                    recovery: if matches!(self.counts.result, NativeResult::Replace) {
                        vec![TransferRecovery {
                            path: destination.path().join("retained-original"),
                            identity: Some(TransferIdentity {
                                device: 1,
                                object: [3; 16],
                            }),
                        }]
                    } else {
                        vec![]
                    },
                    durability_confirmed: !matches!(self.counts.result, NativeResult::Unconfirmed),
                })
            }
        }
    }
}
struct Directory {
    path: PathBuf,
    counts: Arc<Counts>,
}
struct Target(TransferEntry);
impl TransferTarget for Target {
    fn entry(&self) -> &TransferEntry {
        &self.0
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
}
impl TransferDirectory for Directory {
    fn capabilities(&self) -> TransferCapabilities {
        TransferCapabilities {
            recoverable_replacement: !matches!(self.counts.result, NativeResult::Replace),
            ..TransferCapabilities::default()
        }
    }
    fn supports_native_move_replacement(&self, _: TransferIdentity) -> bool {
        matches!(self.counts.result, NativeResult::Replace)
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn identity(&self) -> TransferIdentity {
        TransferIdentity {
            device: 1,
            object: [2; 16],
        }
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn inspect(&self, _: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
        Ok(None)
    }
    fn prepare_target(
        &self,
        name: &OsStr,
        _: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        if matches!(
            self.counts.result,
            NativeResult::SameEntry | NativeResult::Replace
        ) {
            Ok(Some(Arc::new(Target(TransferEntry {
                identity: TransferIdentity {
                    device: 1,
                    object: [if matches!(self.counts.result, NativeResult::SameEntry) {
                        1
                    } else {
                        3
                    }; 16],
                },
                item: MutationItem {
                    path: self.path.join(name),
                    kind: EntryKind::File,
                    metadata: EntryMetadata::default(),
                },
            }))))
        } else {
            Ok(None)
        }
    }
    fn reject_source_ancestor(&self, _: TransferIdentity, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn create_directory(
        &self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Err(unsupported(&self.path))
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.counts.stages.fetch_add(1, Ordering::SeqCst);
        Err(unsupported(&self.path))
    }
}
async fn run(kind: EntryKind, result: NativeResult) -> (Arc<CopyProgress>, Arc<Counts>) {
    let counts = Counts::new(kind, result);
    let root = std::env::current_dir().unwrap();
    let mut harness = Harness::new(Some(Arc::new(Service(counts.clone()))));
    harness
        .controller
        .enqueue_move(request(
            1,
            &[root.join("source")],
            &root.join("destination"),
        ))
        .await;
    let done = harness.terminal(1).await;
    harness.close().await;
    (done, counts)
}
fn assert_no_content(counts: &Counts, done: &CopyProgress) {
    assert_eq!(counts.reads.load(Ordering::SeqCst), 0);
    assert_eq!(counts.lists.load(Ordering::SeqCst), 0);
    assert_eq!(counts.stages.load(Ordering::SeqCst), 0);
    assert_eq!(done.copied_bytes, 0);
    assert_eq!(done.verified_bytes, 0);
    assert_eq!(done.validation_bytes, 0);
    assert_eq!(done.source_files_removed, 0);
}
#[tokio::test]
async fn native_file_move_never_acquires_content_or_proof_authority() {
    let (done, counts) = run(EntryKind::File, NativeResult::Moved).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.native_entries_moved, 1);
    assert_eq!(counts.native.load(Ordering::SeqCst), 1);
    assert_eq!(counts.proof.load(Ordering::SeqCst), 0);
    assert_no_content(&counts, &done);
    assert!(
        done.throughput
            .samples
            .iter()
            .any(|sample| sample.items_per_second > 0.0)
    );
    assert_eq!(done.throughput.items_per_second, 0.0);
}
#[tokio::test]
async fn native_directory_move_does_not_enumerate_descendants() {
    let (done, counts) = run(EntryKind::Directory, NativeResult::Moved).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.native_entries_moved, 1);
    assert_eq!(done.directories_created, 0);
    assert_eq!(counts.proof.load(Ordering::SeqCst), 0);
    assert_no_content(&counts, &done);
}
#[tokio::test]
async fn native_commit_is_retained_when_cancel_arrives_inside_native_call() {
    let (done, counts) = run(EntryKind::File, NativeResult::CancelAfterCommit).await;
    assert_eq!(done.state, CopyOperationState::Cancelled, "{done:?}");
    assert_eq!(done.native_entries_moved, 1);
    assert_no_content(&counts, &done);
}
#[tokio::test]
async fn native_permission_failure_never_falls_through_to_copy() {
    let (done, counts) = run(EntryKind::File, NativeResult::PermissionDenied).await;
    assert_eq!(done.native_entries_moved, 0);
    assert_eq!(counts.native.load(Ordering::SeqCst), 1);
    assert_eq!(counts.proof.load(Ordering::SeqCst), 0);
    assert_eq!(done.errors[0].kind, io::ErrorKind::PermissionDenied);
    assert_no_content(&counts, &done);
}
#[tokio::test]
async fn native_cross_device_fallback_acquires_verified_move_authority() {
    let (done, counts) = run(EntryKind::File, NativeResult::CrossDevice).await;
    assert_eq!(counts.native.load(Ordering::SeqCst), 1);
    assert_eq!(counts.proof.load(Ordering::SeqCst), 1);
    assert_eq!(done.native_entries_moved, 0);
    assert_no_content(&counts, &done);
}

#[tokio::test]
async fn native_unsupported_io_failure_never_becomes_legacy_copy_fallback() {
    let (done, counts) = run(EntryKind::File, NativeResult::UnsupportedIo).await;
    assert_eq!(counts.native.load(Ordering::SeqCst), 1);
    assert_eq!(counts.proof.load(Ordering::SeqCst), 0);
    assert_eq!(done.native_entries_moved, 0);
    assert_eq!(done.errors[0].kind, io::ErrorKind::Unsupported);
    assert_no_content(&counts, &done);
}

#[tokio::test]
async fn native_namespace_durability_unknown_retains_successful_commit() {
    let (done, counts) = run(EntryKind::File, NativeResult::Unconfirmed).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.native_entries_moved, 1);
    assert_eq!(done.source_removal_durability_unconfirmed, 1);
    assert_no_content(&counts, &done);
}

#[tokio::test]
async fn native_same_entry_is_a_noop_without_conflict_or_content_activity() {
    let (done, counts) = run(EntryKind::File, NativeResult::SameEntry).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.native_entries_moved, 0);
    assert_eq!(counts.native.load(Ordering::SeqCst), 1);
    assert_eq!(counts.proof.load(Ordering::SeqCst), 0);
    assert_no_content(&counts, &done);
}

#[tokio::test]
async fn native_replacement_waits_for_exact_recoverable_conflict_consent() {
    let counts = Counts::new(EntryKind::File, NativeResult::Replace);
    let root = std::env::current_dir().unwrap();
    let mut harness = Harness::new(Some(Arc::new(Service(counts.clone()))));
    harness
        .controller
        .enqueue_move(request(
            1,
            &[root.join("source")],
            &root.join("destination"),
        ))
        .await;
    let conflict = harness.conflict(1).await;
    assert!(conflict.can_replace);
    assert_eq!(counts.native.load(Ordering::SeqCst), 0);
    assert_eq!(counts.proof.load(Ordering::SeqCst), 0);
    harness
        .controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::ReplaceRecoverable { all: false },
        })
        .await;
    let done = harness.terminal(1).await;
    harness.close().await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.native_entries_moved, 1);
    assert_eq!(done.recovery_count, 1);
    assert_no_content(&counts, &done);
}

#[tokio::test]
async fn native_mutation_failure_cannot_retry_copy_or_restore_the_capture() {
    for (result, moved) in [
        (NativeResult::RestoredFailure, 0),
        (NativeResult::CommittedFailure, 1),
    ] {
        let (done, counts) = run(EntryKind::File, result).await;
        assert_eq!(done.native_entries_moved, moved);
        assert_eq!(done.failed, 1);
        assert_eq!(counts.native.load(Ordering::SeqCst), 1);
        assert_eq!(counts.proof.load(Ordering::SeqCst), 0);
        assert_no_content(&counts, &done);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_native_move_keeps_capacity_and_reports_late_commit_after_cancel() {
    let counts = Counts::new(EntryKind::File, NativeResult::BlockedCommit);
    let _release = ReleaseOnDrop(counts.released.clone());
    let root = std::env::current_dir().unwrap();
    let mut harness = Harness::new(Some(Arc::new(Service(counts.clone()))));
    harness
        .controller
        .enqueue_move(request(
            1,
            &[root.join("source")],
            &root.join("destination"),
        ))
        .await;
    tokio::time::timeout(Duration::from_secs(4), counts.entered.notified())
        .await
        .unwrap();
    harness.controller.cancel(OperationId(1));
    let waiting = loop {
        if let CopyEvent::Progress(progress) = harness.next().await
            && progress.state == CopyOperationState::CancelRequested
            && !progress.throughput.samples.is_empty()
        {
            break progress;
        }
    };
    assert_eq!(waiting.throughput.items_per_second, 0.0);
    assert_eq!(harness.io.available_permits(), 0);
    assert_eq!(harness.mutation.available_permits(), 0);
    counts.released.store(true, Ordering::Release);
    let done = harness.terminal(1).await;
    harness.close().await;
    assert_eq!(done.state, CopyOperationState::Cancelled, "{done:?}");
    assert_eq!(done.native_entries_moved, 1);
    assert_eq!(done.throughput.items_per_second, 0.0);
    assert!(
        done.throughput
            .samples
            .iter()
            .any(|sample| sample.items_per_second > 0.0)
    );
    assert_no_content(&counts, &done);
}

#[tokio::test]
async fn native_merge_cannot_replace_a_child_moved_earlier_in_the_same_batch() {
    let (_temp, parent) = moves::directories::tree_fixture();
    let first = parent.join("first/tree");
    let second = parent.join("second/tree");
    let destination = parent.join("destination");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::create_dir_all(destination.join("tree")).unwrap();
    fs::write(first.join("file"), b"first native move").unwrap();
    fs::write(second.join("file"), b"later retained source").unwrap();
    let mut harness = Harness::native();
    harness
        .controller
        .enqueue_move(request(1, &[first.clone(), second.clone()], &destination))
        .await;
    let mut file_conflict_can_replace = None;
    let done = loop {
        match harness.next().await {
            CopyEvent::Conflict(conflict) => {
                let choice = if conflict.source.kind == EntryKind::Directory {
                    assert!(conflict.can_merge);
                    CopyChoice::Merge { all: false }
                } else {
                    file_conflict_can_replace = Some(conflict.can_replace);
                    CopyChoice::Skip
                };
                harness
                    .controller
                    .decide(CopyDecision {
                        id: conflict.id,
                        version: conflict.version,
                        choice,
                    })
                    .await;
            }
            CopyEvent::Progress(progress) if progress.state.is_terminal() => break progress,
            CopyEvent::Failure(failure) => panic!("unexpected native merge failure: {failure:?}"),
            _ => {}
        }
    };
    harness.close().await;
    assert_eq!(
        file_conflict_can_replace,
        Some(false),
        "earlier completed Move destinations must remain protected"
    );
    assert_eq!(done.native_entries_moved, 1);
    assert_eq!(done.skipped, 1);
    assert!(!first.exists());
    assert_eq!(
        fs::read(destination.join("tree/file")).unwrap(),
        b"first native move"
    );
    assert_eq!(
        fs::read(second.join("file")).unwrap(),
        b"later retained source"
    );
}
