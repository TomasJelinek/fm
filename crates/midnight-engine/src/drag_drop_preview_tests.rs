use super::*;
use crate::{CommittedSnapshot, SnapshotState};
use midnight_platform::LocalFileSystem;
use std::{fs, path::PathBuf, sync::mpsc as sync_mpsc};

struct Service {
    native: LocalFileSystem,
    destination: PathBuf,
    different_device: bool,
    owner: std::thread::ThreadId,
    blocked: Mutex<Option<(sync_mpsc::SyncSender<()>, sync_mpsc::Receiver<()>)>>,
}
impl TransferService for Service {
    fn native_drop_device(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<u64>, FsError> {
        assert_ne!(self.owner, std::thread::current().id());
        if let Some((entered, release)) = self.blocked.lock().unwrap().take() {
            entered.send(()).unwrap();
            release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        if cancel.is_cancelled() {
            return Err(error(io::ErrorKind::Interrupted, "cancelled"));
        }
        Ok(Some(if self.different_device && path == self.destination {
            2
        } else {
            1
        }))
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        assert_ne!(self.owner, std::thread::current().id());
        self.native.prepare_transfer_source(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        assert_ne!(self.owner, std::thread::current().id());
        self.native.prepare_transfer_destination(path, cancel)
    }
}

struct Harness {
    controller: Controller,
    events: mpsc::Receiver<Event>,
    tasks: JoinSet<()>,
    service: Arc<Service>,
    root: tempfile::TempDir,
}
impl Harness {
    async fn new(different_device: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source 資料");
        let destination = root.path().join("destination");
        fs::create_dir_all(source.join("child")).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("payload"), b"abc").unwrap();
        let entries: Arc<[Entry]> =
            vec![Entry::new(EntryId(1), source.clone(), EntryKind::Directory)].into();
        let snapshots = Arc::new(Mutex::new([
            SnapshotState {
                committed: Some(CommittedSnapshot {
                    navigation: NavigationId(1),
                    path: root.path().into(),
                    entries: entries.clone(),
                    sizes: Arc::default(),
                }),
                ..Default::default()
            },
            SnapshotState {
                committed: Some(CommittedSnapshot {
                    navigation: NavigationId(2),
                    path: destination.clone(),
                    entries: Arc::from([]),
                    sizes: Arc::default(),
                }),
                ..Default::default()
            },
        ]));
        let service = Arc::new(Service {
            native: LocalFileSystem::new(None),
            destination,
            different_device,
            owner: std::thread::current().id(),
            blocked: Mutex::new(None),
        });
        let (sender, events) = mpsc::channel(128);
        let mut tasks = JoinSet::new();
        let mut controller = Controller::new(
            None,
            Some(service.clone()),
            snapshots,
            copy_operations::Admission::default(),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        controller
            .attach(
                DragHost {
                    generation: 1,
                    native: NativeDragWindow::Unsupported,
                    lifetime: Cancellation::default(),
                },
                Arc::default(),
                &mut tasks,
            )
            .await;
        controller
            .prepare(
                DragId(1),
                PaneId::Left,
                NavigationId(1),
                SelectedFiles {
                    selection: Arc::default(),
                    view: Arc::new(DirectoryIndex::from_entries(&entries)),
                    fallback: Some(source),
                },
            )
            .await;
        let mut harness = Self {
            controller,
            events,
            tasks,
            service,
            root,
        };
        loop {
            if matches!(
                harness.next().await,
                DragDropEvent::Prepared { id: DragId(1), .. }
            ) {
                break;
            }
        }
        harness
    }
    fn destination(&self) -> InternalDropDestination {
        InternalDropDestination::PaneRoot {
            pane: PaneId::Right,
            navigation: NavigationId(2),
        }
    }
    async fn preview(
        &self,
        revision: u64,
        destination: InternalDropDestination,
        intent: InternalDropIntent,
    ) -> Cancellation {
        let cancel = Cancellation::default();
        self.controller
            .preview_internal(InternalDropPreviewRequest {
                id: DragId(1),
                revision: DragPreviewRevision(revision),
                destination,
                intent,
                cancel: cancel.clone(),
            })
            .await;
        cancel
    }
    async fn next(&mut self) -> DragDropEvent {
        loop {
            if let Event::DragDrop(event) =
                tokio::time::timeout(Duration::from_secs(5), self.events.recv())
                    .await
                    .unwrap()
                    .unwrap()
            {
                return event;
            }
        }
    }
    async fn result(&mut self) -> (u64, Option<TransferIntent>) {
        loop {
            if let DragDropEvent::InternalPreview {
                revision, intent, ..
            } = self.next().await
            {
                return (revision.0, intent);
            }
        }
    }
    async fn close(mut self) {
        self.controller.shutdown();
        while self.tasks.join_next().await.is_some() {}
    }
}

#[tokio::test]
async fn preview_reports_devices_and_modifiers_without_reserving_or_releasing_sources() {
    for different in [false, true] {
        let mut h = Harness::new(different).await;
        h.preview(1, h.destination(), InternalDropIntent::Auto)
            .await;
        assert_eq!(
            h.result().await,
            (
                1,
                Some(if different {
                    TransferIntent::Copy
                } else {
                    TransferIntent::Move
                })
            )
        );
        h.preview(2, h.destination(), InternalDropIntent::Copy)
            .await;
        assert_eq!(h.result().await, (2, Some(TransferIntent::Copy)));
        h.preview(3, h.destination(), InternalDropIntent::Move)
            .await;
        assert_eq!(h.result().await, (3, Some(TransferIntent::Move)));
        assert!(
            !h.controller
                .state
                .lock()
                .unwrap()
                .source
                .as_ref()
                .unwrap()
                .released
        );
        assert!(h.controller.pending.lock().unwrap().is_empty());
        assert_eq!(h.controller.capacity.capacity().available_permits(), 32);
        assert_eq!(
            fs::read(h.root.path().join("source 資料/payload")).unwrap(),
            b"abc"
        );
        assert!(
            fs::read_dir(&h.service.destination)
                .unwrap()
                .next()
                .is_none()
        );
        h.close().await;
    }
}

#[tokio::test]
async fn preview_rejects_source_itself_parent_self_copy_and_recursive_descendant() {
    let mut h = Harness::new(false).await;
    let source = h.root.path().join("source 資料");
    for (revision, destination) in [
        (1, h.root.path().to_owned()),
        (2, source.clone()),
        (3, source.join("child")),
    ] {
        h.controller.snapshots.lock().unwrap()[1]
            .committed
            .as_mut()
            .unwrap()
            .path = destination;
        h.preview(revision, h.destination(), InternalDropIntent::Copy)
            .await;
        assert_eq!(h.result().await, (revision, None));
    }
    h.close().await;
}

#[tokio::test]
async fn blocked_preview_keeps_only_latest_waiter_and_suppresses_old_receipt() {
    let mut h = Harness::new(false).await;
    let (entered, waiting) = sync_mpsc::sync_channel(1);
    let (release, receiver) = sync_mpsc::sync_channel(1);
    *h.service.blocked.lock().unwrap() = Some((entered, receiver));
    let first = h
        .preview(1, h.destination(), InternalDropIntent::Auto)
        .await;
    tokio::task::spawn_blocking(move || waiting.recv_timeout(Duration::from_secs(3)).unwrap())
        .await
        .unwrap();
    h.preview(2, h.destination(), InternalDropIntent::Move)
        .await;
    h.preview(3, h.destination(), InternalDropIntent::Copy)
        .await;
    assert!(
        !first.is_cancelled(),
        "backend supersession must not cancel the caller's shared input flag"
    );
    assert_eq!(
        h.controller
            .state
            .lock()
            .unwrap()
            .preview
            .as_ref()
            .unwrap()
            .request
            .revision,
        DragPreviewRevision(3)
    );
    release.send(()).unwrap();
    assert_eq!(h.result().await, (3, Some(TransferIntent::Copy)));
    assert!(h.events.try_recv().is_err());
    h.close().await;
}

#[tokio::test]
async fn cancelled_or_navigated_preview_never_reports_success() {
    for navigate in [false, true] {
        let mut h = Harness::new(false).await;
        let (entered, waiting) = sync_mpsc::sync_channel(1);
        let (release, receiver) = sync_mpsc::sync_channel(1);
        *h.service.blocked.lock().unwrap() = Some((entered, receiver));
        let cancel = h
            .preview(1, h.destination(), InternalDropIntent::Auto)
            .await;
        tokio::task::spawn_blocking(move || waiting.recv_timeout(Duration::from_secs(3)).unwrap())
            .await
            .unwrap();
        if navigate {
            h.controller.snapshots.lock().unwrap()[1]
                .committed
                .as_mut()
                .unwrap()
                .navigation = NavigationId(3);
        } else {
            cancel.cancel();
        }
        release.send(()).unwrap();
        if navigate {
            assert_eq!(h.result().await, (1, None));
        }
        h.controller.shutdown();
        while h.tasks.join_next().await.is_some() {}
        while let Ok(Event::DragDrop(event)) = h.events.try_recv() {
            assert!(!matches!(
                event,
                DragDropEvent::InternalPreview {
                    intent: Some(_),
                    ..
                }
            ));
        }
    }
}

#[tokio::test]
async fn release_while_preview_is_pending_revalidates_and_rejects_recursive_drop() {
    let mut h = Harness::new(false).await;
    let (entered, waiting) = sync_mpsc::sync_channel(1);
    let (release, receiver) = sync_mpsc::sync_channel(1);
    *h.service.blocked.lock().unwrap() = Some((entered, receiver));
    h.preview(1, h.destination(), InternalDropIntent::Auto)
        .await;
    tokio::task::spawn_blocking(move || waiting.recv_timeout(Duration::from_secs(3)).unwrap())
        .await
        .unwrap();
    h.controller.snapshots.lock().unwrap()[1]
        .committed
        .as_mut()
        .unwrap()
        .path = h.root.path().join("source 資料/child");
    h.controller
        .internal_request(InternalDropRequest {
            id: DragId(1),
            destination: h.destination(),
            intent: InternalDropIntent::Copy,
            verification: TransferVerification::SizeAndSamples,
        })
        .await;
    release.send(()).unwrap();
    loop {
        match h.next().await {
            DragDropEvent::Failed {
                id: Some(DragId(1)),
                ..
            } => break,
            DragDropEvent::DropAccepted { .. } => {
                panic!("speculative preview must not authorize recursive release")
            }
            DragDropEvent::InternalPreview { .. } => {
                panic!("released preview must not publish a receipt")
            }
            _ => {}
        }
    }
    assert_eq!(h.controller.capacity.capacity().available_permits(), 32);
    h.close().await;
}

#[derive(Debug)]
struct StaleTarget(Arc<Mutex<Option<std::thread::ThreadId>>>);
impl CapturedDropDirectory for StaleTarget {
    fn stamp(&self) -> TreeViewStamp {
        TreeViewStamp {
            pane: PaneId::Right,
            navigation: NavigationId(2),
            session: TreeSessionId(1),
            revision: TreeRevision(1),
        }
    }
    fn row(&self) -> TreeRowId {
        TreeRowId(1)
    }
    fn resolve(&self, _: &Cancellation) -> Result<PathBuf, FsError> {
        panic!("stale capture must never resolve")
    }
}
impl Drop for StaleTarget {
    fn drop(&mut self) {
        *self.0.lock().unwrap() = Some(std::thread::current().id());
    }
}

#[tokio::test]
async fn stale_tree_capture_receipt_is_retired_on_worker_without_cancelling_live_drag() {
    let mut h = Harness::new(false).await;
    h.preview(1, h.destination(), InternalDropIntent::Copy)
        .await;
    assert_eq!(h.result().await, (1, Some(TransferIntent::Copy)));
    let dropped = Arc::new(Mutex::new(None));
    h.controller
        .complete_preview(
            DragId(1),
            DragPreviewRevision(0),
            Arc::new(StaleTarget(dropped.clone())),
        )
        .await;
    h.preview(2, h.destination(), InternalDropIntent::Move)
        .await;
    assert_eq!(h.result().await, (2, Some(TransferIntent::Move)));
    h.close().await;
    assert_ne!(
        dropped.lock().unwrap().unwrap(),
        std::thread::current().id()
    );
}
