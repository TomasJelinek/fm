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

#[path = "operation_summary_tests.rs"]
mod summary;

#[derive(Default)]
struct Calls {
    prepared: AtomicUsize,
    renamed: Mutex<Vec<(PathBuf, String)>>,
    created: Mutex<Vec<PathBuf>>,
    directories: Mutex<Vec<(PathBuf, PathBuf)>>,
    deleted: Mutex<Vec<(PathBuf, DeleteMode)>>,
    blocked: std::sync::atomic::AtomicBool,
    fail_after_progress: std::sync::atomic::AtomicBool,
    cleanup_blocked: std::sync::atomic::AtomicBool,
    cleanup_entered: tokio::sync::Notify,
    entered: tokio::sync::Notify,
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
    fn prepare(&self, path: &Path, _: &Cancellation) -> Result<Arc<dyn PreparedMutation>, FsError> {
        self.0.prepared.fetch_add(1, Ordering::Relaxed);
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
    fn item(&self) -> &MutationItem {
        &self.item
    }
    fn rename(&self, name: &OsStr, _: &Cancellation) -> Result<PathBuf, FsError> {
        self.calls
            .renamed
            .lock()
            .unwrap()
            .push((self.item.path.clone(), name.to_string_lossy().into_owned()));
        Ok(self.item.path.with_file_name(name))
    }
    fn delete(
        &self,
        mode: DeleteMode,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(&Path),
    ) -> Result<(), FsError> {
        self.calls
            .deleted
            .lock()
            .unwrap()
            .push((self.item.path.clone(), mode));
        self.calls.entered.notify_one();
        while self.calls.blocked.load(Ordering::Acquire) && !cancel.is_cancelled() {
            std::thread::park_timeout(Duration::from_millis(5));
        }
        if cancel.is_cancelled() {
            return Err(FsError::from_io(
                "delete",
                &self.item.path,
                std::io::ErrorKind::Interrupted.into(),
            ));
        }
        progress(&self.item.path);
        if self.calls.fail_after_progress.load(Ordering::Acquire) {
            return Err(FsError::from_io(
                "delete",
                &self.item.path,
                std::io::ErrorKind::PermissionDenied.into(),
            ));
        }
        Ok(())
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
        let path = std::env::temp_dir().join("midnight-operation-worker-fixture");
        let entries: Arc<[Entry]> = vec![Entry::new(
            EntryId(7),
            path.join("文档.txt"),
            EntryKind::File,
        )]
        .into();
        let source = VisibleDirectory {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: path.clone(),
            entries: entries.clone(),
        };
        let view = Arc::new(DirectoryIndex::from_entries(&entries));
        let snapshots = Arc::new(Mutex::new([
            SnapshotState {
                archive: None,
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
        ]));
        let (sender, events) = mpsc::channel(64);
        let calls = Arc::new(Calls::default());
        let mut tasks = JoinSet::new();
        let controller = Controller::new(
            Some(Arc::new(FakeService(calls.clone()))),
            snapshots,
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            Arc::new(Semaphore::new(1)),
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
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.calls.cleanup_blocked.store(false, Ordering::Release);
        self.controller.shutdown();
        self.tasks.abort_all();
    }
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
async fn cancelled_confirmation_delivery_can_be_retried_without_losing_the_request() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    let sender = h.controller.sender.clone();
    let capacity: Vec<_> = sender.reserve_many(64).await.unwrap().collect();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            h.controller.confirm(h.request.id, None)
        )
        .await
        .is_err()
    );
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
    let progress = h.finished().await;
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
