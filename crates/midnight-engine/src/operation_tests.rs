use super::*;
use crate::{CommittedSnapshot, SnapshotState};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

#[path = "delete_decision_tests.rs"]
mod decisions;
#[path = "delete_pause_tests.rs"]
mod pause;
#[path = "operation_residue_tests.rs"]
mod residue;
#[path = "operation_retirement_tests.rs"]
mod retirement;
#[path = "operation_summary_tests.rs"]
mod summary;
#[path = "delete_throughput_tests.rs"]
mod throughput;
#[path = "tree_delete_operation_tests.rs"]
mod tree_delete;
#[path = "tree_rename_operation_tests.rs"]
mod tree_rename;

#[derive(Default)]
struct Calls {
    prepared: AtomicUsize,
    prepare_failure: Mutex<Option<FsError>>,
    renamed: Mutex<Vec<(PathBuf, String)>>,
    created: Mutex<Vec<PathBuf>>,
    directories: Mutex<Vec<(PathBuf, PathBuf)>>,
    deleted: Mutex<Vec<(PathBuf, DeleteMode)>>,
    blocked: std::sync::atomic::AtomicBool,
    complete_after_cancel: std::sync::atomic::AtomicBool,
    blocked_after_progress: std::sync::atomic::AtomicBool,
    fail_after_progress: std::sync::atomic::AtomicBool,
    cleanup_blocked: std::sync::atomic::AtomicBool,
    cleanup_entered: tokio::sync::Notify,
    entered: tokio::sync::Notify,
    /// Staging that the next completed rename or delete reports as left behind.
    residue: Mutex<Option<MutationResidue>>,
    /// The error of the next rename, which then does not complete.
    rename_failure: Mutex<Option<FsError>>,
    /// The target whose delete fails after reporting its progress.
    failing_delete: Mutex<Option<PathBuf>>,
    /// Entries each target's discovery reports beyond the target itself, as if
    /// another program removed them before the deletion.
    discovered_extra: AtomicUsize,
    /// Failures a target offers for decision before its deletion completes.
    entry_failures: Mutex<std::collections::HashMap<PathBuf, usize>>,
    /// Their folders became writable: a retried entry is then removed.
    writable: std::sync::atomic::AtomicBool,
    /// Failures offered for decision, including those Skip all errors skipped.
    offered: AtomicUsize,
}
struct FakeService(Arc<Calls>);
struct FakeTarget {
    item: MutationItem,
    calls: Arc<Calls>,
}
impl Drop for FakeTarget {
    fn drop(&mut self) {
        self.calls.cleanup_entered.notify_one();
        while self.calls.cleanup_blocked.load(Ordering::Acquire) {
            std::thread::park_timeout(Duration::from_millis(5));
        }
    }
}

impl MutationService for FakeService {
    fn prepare_create(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn PreparedDirectory>, FsError> {
        self.0.prepared.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(FakeDirectory {
            path: path.into(),
            calls: self.0.clone(),
        }))
    }
    fn prepare(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError> {
        self.0.prepared.fetch_add(1, Ordering::Relaxed);
        let failure = self.0.prepare_failure.lock().unwrap().take();
        if let Some(error) = failure {
            self.0.entered.notify_one();
            while self.0.blocked.load(Ordering::Acquire) && !cancel.is_cancelled() {
                std::thread::park_timeout(Duration::from_millis(5));
            }
            return Err(error);
        }
        Ok(Arc::new(FakeTarget {
            item: MutationItem {
                path: path.into(),
                kind: EntryKind::File,
                metadata: Default::default(),
            },
            calls: self.0.clone(),
        }))
    }
}
struct FakeDirectory {
    path: PathBuf,
    calls: Arc<Calls>,
}
impl PreparedDirectory for FakeDirectory {
    fn create(&self, path: &Path, _: &Cancellation) -> Result<PathBuf, FsError> {
        self.calls
            .directories
            .lock()
            .unwrap()
            .push((self.path.clone(), path.to_owned()));
        Ok(self.path.join(path))
    }
    fn create_file(&self, name: &OsStr, _: &Cancellation) -> Result<PathBuf, FsError> {
        if self.calls.fail_after_progress.load(Ordering::Acquire) {
            return Err(FsError::from_io(
                "create file",
                &self.path,
                std::io::ErrorKind::AlreadyExists.into(),
            ));
        }
        let path = self.path.join(name);
        self.calls.created.lock().unwrap().push(path.clone());
        Ok(path)
    }
}
impl PreparedMutation for FakeTarget {
    /// The trait's default, plus `Calls::discovered_extra` entries after a file.
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        let refused = if !cancel.wait_if_paused() {
            Some(std::io::ErrorKind::Interrupted)
        } else {
            (self.item.kind == EntryKind::Directory).then_some(std::io::ErrorKind::Unsupported)
        };
        if let Some(kind) = refused {
            return Err(FsError::from_io(
                "discover operation",
                &self.item.path,
                kind.into(),
            ));
        }
        visit(&self.item);
        for index in 0..self.calls.discovered_extra.load(Ordering::Relaxed) {
            visit(&MutationItem {
                path: self.item.path.join(index.to_string()),
                ..self.item.clone()
            });
        }
        Ok(())
    }
    fn item(&self) -> &MutationItem {
        &self.item
    }
    fn rename(&self, name: &OsStr, _: &Cancellation) -> Result<RenameCompletion, FsError> {
        self.calls
            .renamed
            .lock()
            .unwrap()
            .push((self.item.path.clone(), name.to_string_lossy().into_owned()));
        if let Some(failure) = self.calls.rename_failure.lock().unwrap().take() {
            return Err(failure);
        }
        Ok(RenameCompletion {
            destination: self.item.path.with_file_name(name),
            residue: self.calls.residue.lock().unwrap().take(),
        })
    }
    fn delete(
        &self,
        mode: DeleteMode,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(&Path),
    ) -> Result<DeleteCompletion, FsError> {
        self.calls
            .deleted
            .lock()
            .unwrap()
            .push((self.item.path.clone(), mode));
        self.calls.entered.notify_one();
        while self.calls.blocked.load(Ordering::Acquire)
            && (!cancel.is_cancelled() || self.calls.complete_after_cancel.load(Ordering::Acquire))
        {
            std::thread::park_timeout(Duration::from_millis(5));
        }
        let interrupted = || {
            FsError::from_io(
                "delete",
                &self.item.path,
                std::io::ErrorKind::Interrupted.into(),
            )
        };
        if cancel.is_cancelled() && !self.calls.complete_after_cancel.load(Ordering::Acquire) {
            return Err(interrupted());
        }
        progress(&self.item.path);
        while self.calls.blocked_after_progress.load(Ordering::Acquire) && !cancel.is_cancelled() {
            std::thread::park_timeout(Duration::from_millis(5));
        }
        if self.calls.fail_after_progress.load(Ordering::Acquire)
            || self.calls.failing_delete.lock().unwrap().as_ref() == Some(&self.item.path)
        {
            return Err(FsError::from_io(
                "delete",
                &self.item.path,
                std::io::ErrorKind::PermissionDenied.into(),
            ));
        }
        if cancel.is_cancelled() && !self.calls.complete_after_cancel.load(Ordering::Acquire) {
            return Err(interrupted());
        }
        Ok(DeleteCompletion {
            residue: self.calls.residue.lock().unwrap().take(),
            skipped: 0,
        })
    }
    /// Offers `Calls::entry_failures` failures for this target first, each for one
    /// entry below it; Retry succeeds once `Calls::writable` is set.
    fn delete_with_decisions(
        &self,
        mode: DeleteMode,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(&Path),
        failed: &mut dyn FnMut(DeleteEntryFailure) -> DeleteFailureChoice,
    ) -> Result<DeleteCompletion, FsError> {
        let failures = self
            .calls
            .entry_failures
            .lock()
            .unwrap()
            .get(&self.item.path)
            .copied()
            .unwrap_or(0);
        let mut skipped = 0;
        for index in 0..failures {
            let entry = self.item.path.join(format!("locked-{index}"));
            loop {
                if self.calls.writable.load(Ordering::Acquire) {
                    progress(&entry);
                    break;
                }
                self.calls.offered.fetch_add(1, Ordering::AcqRel);
                match failed(DeleteEntryFailure {
                    error: FsError::from_io(
                        "mutate entry",
                        &entry,
                        std::io::ErrorKind::PermissionDenied.into(),
                    ),
                    can_retry: true,
                }) {
                    DeleteFailureChoice::Retry => {}
                    DeleteFailureChoice::Skip => {
                        skipped += 1;
                        break;
                    }
                    DeleteFailureChoice::Cancel => {
                        return Err(FsError::from_io(
                            "mutate entry",
                            &entry,
                            std::io::ErrorKind::Interrupted.into(),
                        ));
                    }
                }
            }
        }
        if skipped > 0 {
            return Ok(DeleteCompletion {
                residue: None,
                skipped,
            });
        }
        self.delete(mode, cancel, progress)
    }
}

struct Harness {
    controller: Controller,
    calls: Arc<Calls>,
    events: mpsc::Receiver<Event>,
    tasks: JoinSet<()>,
    request: PrepareOperation,
}
impl Harness {
    fn new(kind: MutationKind) -> Self {
        Self::using(kind, None)
    }
    fn using(kind: MutationKind, service: Option<Arc<dyn MutationService>>) -> Self {
        let path = std::env::temp_dir().join("midnight-operation-worker-fixture");
        let entries: Arc<[Entry]> = vec![Entry::new(
            EntryId(7),
            path.join("文档.txt"),
            EntryKind::File,
        )]
        .into();
        Self::in_directory(kind, service, path, entries)
    }

    fn in_directory(
        kind: MutationKind,
        service: Option<Arc<dyn MutationService>>,
        path: PathBuf,
        entries: Arc<[Entry]>,
    ) -> Self {
        Self::serving(kind, service, None, path, entries)
    }

    /// The local platform's mutation and transfer services on a real folder.
    fn native(kind: MutationKind, path: &Path, entries: Arc<[Entry]>) -> Self {
        let local = Arc::new(midnight_platform::LocalFileSystem::new(None));
        Self::serving(
            kind,
            Some(local.clone()),
            Some(local),
            path.to_owned(),
            entries,
        )
    }

    fn serving(
        kind: MutationKind,
        service: Option<Arc<dyn MutationService>>,
        transfer: Option<Arc<dyn TransferService>>,
        path: PathBuf,
        entries: Arc<[Entry]>,
    ) -> Self {
        let source = VisibleDirectory {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: path.clone(),
            entries: entries.clone(),
        };
        let view = Arc::new(DirectoryIndex::from_entries(&entries));
        let snapshots = crate::test_snapshots([
            SnapshotState {
                archive: None,
                network: None,
                network_navigation: None,
                archive_navigation: None,
                visible: Some(CommittedSnapshot {
                    navigation: source.navigation,
                    path,
                    entries,
                    sizes: Default::default(),
                }),
                ..Default::default()
            },
            SnapshotState::default(),
        ]);
        let (sender, events) = mpsc::channel(64);
        let calls = Arc::new(Calls::default());
        let mut tasks = JoinSet::new();
        let controller = Controller::new(
            crate::worker_retirement::ActorRetirement::testing(),
            Services {
                mutation: Some(service.unwrap_or_else(|| Arc::new(FakeService(calls.clone())))),
                locale: None,
                transfer,
            },
            snapshots,
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            crate::transfer_leases::namespace_gate(),
            CopyFence::default(),
            &mut tasks,
        );
        Self {
            controller,
            calls,
            events,
            tasks,
            request: PrepareOperation {
                id: OperationId(1),
                kind,
                source,
                view,
                selection: Arc::default(),
                focused: Some(EntryId(7)),
            },
        }
    }
    async fn next(&mut self) -> OperationEvent {
        match tokio::time::timeout(Duration::from_secs(2), self.events.recv())
            .await
            .expect("operation worker must respond")
            .unwrap()
        {
            Event::Operation(event) => event,
            event => panic!("unexpected event: {event:?}"),
        }
    }
    async fn preview(&mut self) -> Arc<OperationPreview> {
        loop {
            if let OperationEvent::Preview(preview) = self.next().await {
                return preview;
            }
        }
    }
    async fn finished(&mut self) -> Arc<OperationProgress> {
        loop {
            if let OperationEvent::Progress(progress) = self.next().await
                && progress.state.is_terminal()
            {
                return progress;
            }
        }
    }
    /// The terminal progress, answering each Delete failure on the way with `choice`,
    /// and the failures it answered.
    async fn finished_deciding(
        &mut self,
        choice: CopyChoice,
    ) -> (Arc<OperationProgress>, Vec<Arc<DeleteFailure>>) {
        let mut asked = Vec::new();
        loop {
            match self.next().await {
                OperationEvent::Progress(progress) if progress.state.is_terminal() => {
                    return (progress, asked);
                }
                OperationEvent::DeleteFailure(failure) => {
                    self.controller
                        .decide_delete(CopyDecision {
                            id: failure.id,
                            version: failure.version,
                            choice: choice.clone(),
                        })
                        .await;
                    asked.push(failure);
                }
                _ => {}
            }
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.calls.blocked.store(false, Ordering::Release);
        self.calls
            .blocked_after_progress
            .store(false, Ordering::Release);
        self.calls.cleanup_blocked.store(false, Ordering::Release);
        self.controller.shutdown();
        self.tasks.abort_all();
    }
}

#[tokio::test(start_paused = true)]
async fn idle_actor_exits_on_shutdown_without_a_polling_timer() {
    let mut h = Harness::new(MutationKind::Rename);
    tokio::task::yield_now().await; // The actor reaches its idle wait.
    // Paused time advances only to fire a timer: an idle actor must need none.
    let started = tokio::time::Instant::now();
    h.controller.shutdown();
    let exited = tokio::time::timeout(Duration::from_secs(5), h.tasks.join_next()).await;
    assert!(matches!(exited, Ok(Some(Ok(())))), "{exited:?}");
    assert_eq!(tokio::time::Instant::now(), started);
}

#[tokio::test]
async fn failed_actor_task_still_reports_its_registered_job() {
    let h = Harness::new(MutationKind::Rename);
    let status = Arc::new(Mutex::new(worker::initial(&h.request)));
    let mut tasks = Tracked::<()>::default();
    tasks.spawn(status.clone(), async {
        panic!("injected actor task failure")
    });
    tasks.spawn(status, async {});
    let (mut completed, mut failed) = (0, Vec::new());
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(()) => completed += 1,
            Err(progress) => failed.push(progress),
        }
    }
    assert_eq!(completed, 1);
    assert_eq!(failed.len(), 1, "a panicked task must not vanish");
    assert_eq!(failed[0].id, h.request.id);
    assert_eq!(failed[0].state, OperationState::Failed);
    assert!(failed[0].message.contains("injected actor task failure"));
    assert!(tasks.is_empty() && tasks.jobs.is_empty());
}

#[tokio::test]
async fn cancelled_job_retains_its_budget_until_backend_cleanup_finishes() {
    let mut h = Harness::new(MutationKind::Rename);
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.calls.cleanup_blocked.store(true, Ordering::Release);
    h.controller.cancel(h.request.id);
    tokio::time::timeout(Duration::from_secs(2), h.calls.cleanup_entered.notified())
        .await
        .unwrap();
    let registered = h
        .controller
        .registry
        .lock()
        .unwrap()
        .contains_key(&h.request.id);
    // Always release the fixture before asserting, including on the RED path.
    h.calls.cleanup_blocked.store(false, Ordering::Release);
    assert!(
        registered,
        "Native resources still occupy the bounded job slot"
    );
    assert_eq!(h.finished().await.state, OperationState::Cancelled);
}

#[tokio::test]
async fn delete_preparation_interruption_requires_a_cancel_request_to_be_cancelled() {
    for cancel_requested in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join(".claude");
        std::fs::create_dir(&target).unwrap();
        let payload = target.join("fixture.txt");
        std::fs::write(&payload, b"keep this file").unwrap();
        let mut h = Harness::in_directory(
            MutationKind::Delete(DeleteMode::Trash),
            None,
            directory.path().to_owned(),
            Arc::from([Entry::new(EntryId(7), target.clone(), EntryKind::Directory)]),
        );
        *h.calls.prepare_failure.lock().unwrap() = Some(FsError::from_io(
            "prepare delete",
            &target,
            std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "fixture preparation was interrupted",
            ),
        ));
        h.calls.blocked.store(cancel_requested, Ordering::Release);

        h.controller.prepare(h.request.clone()).await;
        if cancel_requested {
            tokio::time::timeout(Duration::from_secs(2), h.calls.entered.notified())
                .await
                .unwrap();
            h.controller.cancel(h.request.id);
        }
        let progress = h.finished().await;
        let expected = if cancel_requested {
            OperationState::Cancelled
        } else {
            OperationState::Failed
        };
        assert_eq!(progress.state, expected);
        assert_eq!(
            (progress.completed, progress.failed),
            (0, u64::from(!cancel_requested))
        );
        assert_eq!(progress.errors.len(), 1);
        assert_eq!(progress.errors[0].kind, std::io::ErrorKind::Interrupted);
        assert!(progress.message.contains(".claude"));
        assert!(
            progress
                .message
                .contains("fixture preparation was interrupted")
        );
        assert!(h.calls.deleted.lock().unwrap().is_empty());
        assert_eq!(std::fs::read(payload).unwrap(), b"keep this file");
    }
}

#[tokio::test]
async fn cancelled_confirmation_never_mutates_and_stale_snapshot_is_rejected() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller.cancel(h.request.id);
    assert_eq!(h.finished().await.state, OperationState::Cancelled);
    h.controller.confirm(h.request.id, None).await;
    h.request.id = OperationId(2);
    h.request.source.navigation = NavigationId(99);
    h.controller.prepare(h.request.clone()).await;
    assert_eq!(h.finished().await.state, OperationState::Failed);
    assert!(h.calls.deleted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn virtual_views_never_prepare_mutations_against_retained_temp_paths() {
    for kind in [
        MutationKind::CreateFile,
        MutationKind::CreateDirectory,
        MutationKind::Rename,
        MutationKind::EditMetadata,
        MutationKind::Delete(DeleteMode::Trash),
        MutationKind::Delete(DeleteMode::Permanent),
    ] {
        for state in 0..4 {
            let mut h = Harness::new(kind);
            {
                let mut snapshots = h.controller.snapshots.lock().unwrap();
                let snapshot = &mut snapshots[0];
                match state {
                    0 => snapshot.archive_navigation = Some(NavigationId(2)),
                    1 => snapshot.archive = Some((NavigationId(2), Arc::from([]))),
                    2 => snapshot.network_navigation = Some(NavigationId(2)),
                    3 => snapshot.network = Some((NavigationId(2), Arc::from([]))),
                    _ => unreachable!(),
                }
                // The old TEMP snapshot is deliberately retained and still exactly
                // matches the request. Provider authority must override that match.
            }
            h.controller.prepare(h.request.clone()).await;
            assert_eq!(
                h.finished().await.state,
                OperationState::Failed,
                "{kind:?}/{state}"
            );
            h.controller.confirm(h.request.id, None).await;
            assert_eq!(
                h.calls.prepared.load(Ordering::Relaxed),
                0,
                "{kind:?}/{state}"
            );
            assert!(h.calls.deleted.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn cancelled_confirmation_delivery_can_be_retried_without_losing_the_request() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    let sender = h.controller.sender.clone();
    let capacity: Vec<_> = sender.reserve_many(64).await.unwrap().collect();
    {
        let delivery = h.controller.confirm(h.request.id, None);
        tokio::pin!(delivery);
        // One poll reaches the full queue; dropping the pending delivery cancels it.
        tokio::select! {
            biased;
            () = &mut delivery => panic!("a full queue cannot accept the confirmation"),
            () = std::future::ready(()) => {}
        }
    }
    drop(capacity);
    h.controller.confirm(h.request.id, None).await;
    assert_eq!(h.finished().await.state, OperationState::Completed);
    assert_eq!(h.calls.deleted.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn mutations_are_serial_and_cancelling_one_does_not_cancel_the_next() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Trash));
    h.calls.blocked.store(true, Ordering::Release);
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller.confirm(h.request.id, None).await;
    tokio::time::timeout(Duration::from_secs(2), h.calls.entered.notified())
        .await
        .unwrap();
    h.request.id = OperationId(2);
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller.confirm(h.request.id, None).await;
    loop {
        if let OperationEvent::Progress(progress) = h.next().await
            && progress.id == OperationId(2)
            && progress.state == OperationState::Queued
        {
            break;
        }
    }
    assert_eq!(h.calls.deleted.lock().unwrap().len(), 1);
    h.controller.cancel(OperationId(1));
    let first = h.finished().await;
    assert_eq!(
        (first.id, first.state),
        (OperationId(1), OperationState::Cancelled)
    );
    h.calls.blocked.store(false, Ordering::Release);
    let second = h.finished().await;
    assert_eq!(
        (second.id, second.state),
        (OperationId(2), OperationState::Completed)
    );
    assert_eq!(h.calls.deleted.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn cancelled_delete_preserves_partial_progress_and_only_genuine_errors() {
    for fail_after_progress in [false, true] {
        let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
        h.calls
            .blocked_after_progress
            .store(true, Ordering::Release);
        h.calls
            .fail_after_progress
            .store(fail_after_progress, Ordering::Release);
        h.controller.prepare(h.request.clone()).await;
        h.preview().await;
        h.controller.confirm(h.request.id, None).await;

        // The provider has already removed one entry, but has not returned
        // from deleting the selected root.
        loop {
            if let OperationEvent::Progress(progress) = h.next().await
                && progress.state == OperationState::Running
                && progress.processed_entries == 1
            {
                break;
            }
        }
        h.controller.cancel(h.request.id);
        let done = h.finished().await;
        assert_eq!(done.state, OperationState::Cancelled);
        assert_eq!((done.completed, done.processed_entries), (0, 1));
        assert_eq!(done.failed, u64::from(fail_after_progress));
        assert!(done.message.contains("have not been undone"));
        if fail_after_progress {
            assert_eq!(done.errors.len(), 1);
            assert_eq!(done.errors[0].kind, std::io::ErrorKind::PermissionDenied);
            assert_eq!(done.errors[0].path, h.request.source.entries[0].path);
        } else {
            assert!(done.errors.is_empty());
        }
    }
}

#[tokio::test]
async fn prepared_target_resources_are_bounded() {
    let mut h = Harness::new(MutationKind::Rename);
    for id in 1..=MAX_JOBS as u64 {
        h.request.id = OperationId(id);
        h.controller.prepare(h.request.clone()).await;
        assert_eq!(h.preview().await.id, OperationId(id));
    }
    h.request.id = OperationId(MAX_JOBS as u64 + 1);
    h.controller.prepare(h.request.clone()).await;
    assert_eq!(h.finished().await.state, OperationState::Failed);
    assert_eq!(h.calls.prepared.load(Ordering::Relaxed), MAX_JOBS);
    assert!(h.calls.renamed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn partial_recursive_failure_reports_changes_and_original_target_after_navigation() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
    h.calls.fail_after_progress.store(true, Ordering::Release);
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller.snapshots.lock().unwrap()[0].visible = None;
    h.controller.confirm(h.request.id, None).await;
    // The failure that ended the target waits for a decision; Skip goes on.
    let (progress, asked) = h.finished_deciding(CopyChoice::Skip).await;
    assert_eq!(asked.len(), 1);
    assert!(asked[0].can_skip);
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!(
        (
            progress.completed,
            progress.failed,
            progress.processed_entries
        ),
        (0, 1, 1)
    );
    assert_eq!(progress.errors.len(), 1);
    assert_eq!(
        h.calls.deleted.lock().unwrap()[0].0,
        h.request.source.entries[0].path
    );
}

#[tokio::test]
async fn preparation_does_not_mutate_and_confirmation_runs_exactly_once() {
    let mut h = Harness::new(MutationKind::Rename);
    h.controller.prepare(h.request.clone()).await;
    let preview = h.preview().await;
    assert_eq!(preview.files, 1);
    assert_eq!(preview.sample[0].path, h.request.source.entries[0].path);
    assert!(h.calls.renamed.lock().unwrap().is_empty());
    h.controller
        .confirm(h.request.id, Some("new 文档.txt".into()))
        .await;
    assert_eq!(h.finished().await.state, OperationState::Completed);
    h.controller
        .confirm(h.request.id, Some("should-not-run.txt".into()))
        .await;
    h.controller.prepare(h.request.clone()).await;
    h.request.id = OperationId(2);
    h.controller.prepare(h.request.clone()).await;
    assert_eq!(h.preview().await.id, OperationId(2));
    assert_eq!(h.calls.prepared.load(Ordering::Relaxed), 2);
    assert_eq!(h.calls.renamed.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn delete_confirmation_preserves_the_explicit_mode() {
    for mode in [DeleteMode::Trash, DeleteMode::Permanent] {
        let mut h = Harness::new(MutationKind::Delete(mode));
        h.controller.prepare(h.request.clone()).await;
        h.preview().await;
        assert!(h.calls.deleted.lock().unwrap().is_empty());
        h.controller.confirm(h.request.id, None).await;
        let progress = h.finished().await;
        assert_eq!(progress.state, OperationState::Completed);
        assert_eq!(progress.completed, 1);
        assert_eq!(progress.processed_entries, 1);
        assert_eq!(h.calls.deleted.lock().unwrap()[0].1, mode);
    }
}

#[tokio::test]
async fn create_file_uses_captured_directory_without_focus_and_confirms_exactly_once() {
    let mut h = Harness::new(MutationKind::CreateFile);
    h.request.focused = None;
    h.controller.prepare(h.request.clone()).await;
    let preview = h.preview().await;
    assert_eq!((preview.files, preview.folders), (0, 0));
    assert!(preview.sample.is_empty());
    assert!(h.calls.created.lock().unwrap().is_empty());
    h.controller.snapshots.lock().unwrap()[0].visible = None;
    h.controller
        .confirm(h.request.id, Some("文档 🦀.txt".into()))
        .await;
    let progress = h.finished().await;
    let expected = h.request.source.path.join("文档 🦀.txt");
    assert_eq!(progress.state, OperationState::Completed);
    assert_eq!(
        (
            progress.total,
            progress.completed,
            progress.processed_entries
        ),
        (1, 1, 1)
    );
    assert_eq!(progress.renamed_to.as_ref(), Some(&expected));
    h.controller
        .confirm(h.request.id, Some("duplicate.txt".into()))
        .await;
    // A following completed job is a barrier proving the duplicate was consumed.
    h.controller.snapshots.lock().unwrap()[0].visible = Some(CommittedSnapshot {
        navigation: h.request.source.navigation,
        path: h.request.source.path.clone(),
        entries: h.request.source.entries.clone(),
        sizes: Default::default(),
    });
    h.request.id = OperationId(2);
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller.cancel(h.request.id);
    assert_eq!(h.finished().await.state, OperationState::Cancelled);
    assert_eq!(h.calls.created.lock().unwrap().as_slice(), &[expected]);
    assert!(h.calls.directories.lock().unwrap().is_empty());
}

#[tokio::test]
async fn create_directory_passes_explicit_paths_to_the_captured_native_base_after_navigation() {
    for name in [
        "../sibling/文档",
        "/absolute/new",
        "C:\\absolute\\new",
        "\\\\server\\share\\new",
    ] {
        let mut h = Harness::new(MutationKind::CreateDirectory);
        h.request.focused = None;
        h.controller.prepare(h.request.clone()).await;
        h.preview().await;
        h.controller.snapshots.lock().unwrap()[0].visible = None;
        h.controller.confirm(h.request.id, Some(name.into())).await;
        assert_eq!(
            h.finished().await.state,
            OperationState::Completed,
            "{name}"
        );
        assert_eq!(
            h.calls.directories.lock().unwrap().as_slice(),
            &[(h.request.source.path.clone(), PathBuf::from(name))]
        );
        assert!(h.calls.created.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn create_file_invalid_confirmation_and_collisions_do_not_report_success() {
    for name in [
        None,
        Some(""),
        Some("../escape"),
        Some("nested/file"),
        Some("nul\0name"),
    ] {
        let mut h = Harness::new(MutationKind::CreateFile);
        h.controller.prepare(h.request.clone()).await;
        h.preview().await;
        h.controller
            .confirm(h.request.id, name.map(str::to_owned))
            .await;
        assert_eq!(h.finished().await.state, OperationState::Failed);
        assert!(h.calls.created.lock().unwrap().is_empty());
    }
    let mut h = Harness::new(MutationKind::CreateFile);
    h.calls.fail_after_progress.store(true, Ordering::Release);
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller
        .confirm(h.request.id, Some("existing".into()))
        .await;
    let result = h.finished().await;
    assert_eq!(result.state, OperationState::Failed);
    assert_eq!(
        (result.completed, result.failed, result.processed_entries),
        (0, 1, 0)
    );
    assert_eq!(result.errors[0].kind, std::io::ErrorKind::AlreadyExists);
    assert!(h.calls.created.lock().unwrap().is_empty());
}

/// A real folder holding `name` with `contents`, and a New empty file job prepared there.
async fn prepared_replacement(folder: &Path, name: &str, contents: &[u8]) -> (Harness, PathBuf) {
    let existing = folder.join(name);
    std::fs::write(&existing, contents).unwrap();
    let mut h = Harness::native(
        MutationKind::CreateFile,
        folder,
        Arc::from([Entry::new(EntryId(7), existing.clone(), EntryKind::File)]),
    );
    h.request.focused = None;
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    (h, existing)
}

#[tokio::test]
async fn a_confirmed_replacement_keeps_the_original_and_says_where() {
    let folder = tempfile::tempdir().unwrap();
    let (mut h, existing) =
        prepared_replacement(folder.path(), "video 文档.mkv", b"original contents").await;
    h.controller
        .confirm_with(h.request.id, Some("video 文档.mkv".into()), true)
        .await;
    let progress = h.finished().await;
    assert_eq!(
        progress.state,
        OperationState::Completed,
        "{}",
        progress.message
    );
    assert_eq!(progress.renamed_to.as_deref(), Some(existing.as_path()));
    assert_eq!(std::fs::read(&existing).unwrap(), b"");
    // The completion names where the original is kept, and it is there unchanged.
    let kept = progress
        .message
        .lines()
        .find(|line| line.contains(".midnight-recovery-"))
        .unwrap_or_else(|| panic!("no recovery path in {:?}", progress.message));
    assert_eq!(std::fs::read(kept).unwrap(), b"original contents");
    assert!(
        progress
            .message
            .contains("Delete it when you no longer need it."),
        "{}",
        progress.message
    );
}

#[tokio::test]
async fn a_replacement_never_follows_its_folder_path_to_another_folder() {
    let parent = tempfile::tempdir().unwrap();
    let folder = parent.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    let (mut h, existing) = prepared_replacement(&folder, "a.txt", b"pinned").await;
    // Another program moves the prepared folder away and puts a new one at its path.
    let moved = parent.path().join("moved");
    std::fs::rename(&folder, &moved).unwrap();
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(&existing, b"newcomer").unwrap();
    h.controller
        .confirm_with(h.request.id, Some("a.txt".into()), true)
        .await;
    let progress = h.finished().await;
    assert_eq!(progress.state, OperationState::Failed);
    assert!(
        progress.message.contains("moved or replaced"),
        "{}",
        progress.message
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"newcomer");
    assert_eq!(std::fs::read(moved.join("a.txt")).unwrap(), b"pinned");
}

#[tokio::test]
async fn a_replacement_waits_for_copies_outstanding_when_it_is_confirmed() {
    let folder = tempfile::tempdir().unwrap();
    let (mut h, existing) = prepared_replacement(folder.path(), "a.txt", b"original").await;
    // A copy still running at confirmation may be writing this very file.
    let copy = h.controller.copies.register();
    h.controller
        .confirm_with(h.request.id, Some("a.txt".into()), true)
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(300), h.finished())
            .await
            .is_err(),
        "the replacement must wait for the earlier copy"
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"original");
    // That copy ends without a verified publication: the replacement is refused.
    drop(copy);
    let progress = h.finished().await;
    assert_eq!(progress.state, OperationState::Failed);
    assert!(
        progress.message.contains("earlier copy"),
        "{}",
        progress.message
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"original");
}
