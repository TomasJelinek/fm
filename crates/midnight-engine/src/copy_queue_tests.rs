use super::*;
use crate::{CommittedSnapshot, SnapshotState, operations};

struct Mutations {
    controller: operations::Controller,
    events: mpsc::Receiver<Event>,
    request: PrepareOperation,
}

impl Mutations {
    fn new(h: &mut Harness, source: &Path) -> Self {
        let entries: Arc<[Entry]> =
            vec![Entry::new(EntryId(1), source.to_owned(), EntryKind::File)].into();
        let directory = source.parent().unwrap().to_owned();
        let visible = VisibleDirectory {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: directory.clone(),
            entries: entries.clone(),
        };
        let snapshots = Arc::new(Mutex::new([
            SnapshotState {
                visible: Some(CommittedSnapshot {
                    navigation: visible.navigation,
                    path: directory,
                    entries: entries.clone(),
                    sizes: Default::default(),
                }),
                ..Default::default()
            },
            SnapshotState::default(),
        ]));
        let (sender, events) = mpsc::channel(64);
        let controller = operations::Controller::new(
            Some(Arc::new(midnight_platform::LocalFileSystem::new(None))),
            snapshots,
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            h.mutation.clone(),
            h.controller.admission().fence(),
            &mut h.tasks,
        );
        Self {
            controller,
            events,
            request: PrepareOperation {
                id: OperationId(50),
                kind: MutationKind::Delete(DeleteMode::Permanent),
                source: visible,
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                selection: Arc::default(),
                focused: Some(EntryId(1)),
            },
        }
    }

    async fn confirm(&mut self) {
        self.controller.prepare(self.request.clone()).await;
        while !matches!(self.next().await, OperationEvent::Preview(_)) {}
        self.controller.confirm(self.request.id, None).await;
        while !matches!(self.next().await, OperationEvent::Progress(p) if p.state == OperationState::Queued)
        {
        }
    }

    async fn next(&mut self) -> OperationEvent {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if let Some(Event::Operation(event)) = self.events.recv().await {
                    return event;
                }
            }
        })
        .await
        .unwrap()
    }

    async fn terminal(&mut self) -> Arc<OperationProgress> {
        loop {
            if let OperationEvent::Progress(progress) = self.next().await
                && progress.state.is_terminal()
            {
                return progress;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_delete_waits_for_every_earlier_copy_before_removing_its_source() {
    let temp = fixture();
    let first = temp.path().join("first");
    let source = temp.path().join("last original");
    let target = temp.path().join("target");
    fs::write(&first, b"first").unwrap();
    fs::write(&source, b"irreplaceable source").unwrap();
    fs::create_dir(&target).unwrap();
    let entered = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let _release = ReleaseOnDrop(released.clone());
    let mut h = Harness::new(Some(Arc::new(BlockingService {
        inner: midnight_platform::LocalFileSystem::new(None),
        entered: entered.clone(),
        released: released.clone(),
        prepared: Arc::new(AtomicUsize::new(0)),
        read_before_block: false,
    })));
    h.controller.enqueue(request(1, &[first], &target)).await;
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();
    h.controller
        .enqueue(request(2, std::slice::from_ref(&source), &target))
        .await;
    let mut mutations = Mutations::new(&mut h, &source);
    mutations.confirm().await;
    // Give the confirmed mutation time to request the gate while A owns it.
    tokio::time::sleep(Duration::from_millis(80)).await;
    released.store(true, Ordering::Release);
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Completed);
    let second = h.terminal(2).await;
    let deletion = mutations.terminal().await;
    mutations.controller.shutdown();
    h.close().await;
    assert_eq!(second.state, CopyOperationState::Completed, "{second:?}");
    assert_eq!(deletion.state, OperationState::Completed);
    assert_eq!(
        fs::read(target.join("last original")).unwrap(),
        b"irreplaceable source"
    );
    assert!(!source.exists());
}

#[tokio::test]
async fn native_reservation_keeps_fifo_before_its_ui_payload_arrives() {
    let temp = fixture();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::write(&source, b"source").unwrap();
    fs::create_dir(&target).unwrap();
    let mut h = Harness::native();
    let reservation = h.controller.reserve().unwrap();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &target))
        .await;
    assert!(
        matches!(h.next().await, CopyEvent::Progress(p) if p.state == CopyOperationState::Queued)
    );
    let overtook = tokio::time::timeout(Duration::from_millis(200), h.next())
        .await
        .is_ok();
    drop(reservation);
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Completed);
    h.close().await;
    assert!(
        !overtook,
        "later copy must wait for earlier native reservation"
    );
}

#[tokio::test]
async fn incomplete_copy_predecessors_preserve_the_source_and_require_reconfirmation() {
    for outcome in ["failed", "cancelled", "skipped", "abandoned", "rejected"] {
        let temp = fixture();
        let source = temp.path().join("original");
        let target = temp.path().join("target");
        fs::write(&source, b"keep the original").unwrap();
        fs::create_dir(&target).unwrap();
        let mut h = Harness::native();
        let reservation = h.controller.reserve().unwrap();
        let mut mutations = Mutations::new(&mut h, &source);
        mutations.confirm().await;
        let gate = if outcome == "cancelled" {
            Some(h.mutation.clone().acquire_owned().await.unwrap())
        } else {
            None
        };
        if outcome == "abandoned" {
            drop(reservation);
        } else {
            if outcome == "skipped" {
                fs::write(target.join("original"), b"existing").unwrap();
            }
            let destination = match outcome {
                "failed" => temp.path().join("missing target"),
                "rejected" => PathBuf::from("relative"),
                _ => target.clone(),
            };
            h.controller
                .enqueue_reserved(
                    request(1, std::slice::from_ref(&source), &destination),
                    reservation,
                )
                .await;
            if outcome == "cancelled" {
                h.controller.cancel(OperationId(1));
            }
            if outcome == "skipped" {
                loop {
                    if let CopyEvent::Conflict(conflict) = h.next().await {
                        h.controller
                            .decide(CopyDecision {
                                id: conflict.id,
                                version: conflict.version,
                                choice: CopyChoice::Skip,
                            })
                            .await;
                        break;
                    }
                }
            }
            drop(gate);
            assert_ne!(h.terminal(1).await.state, CopyOperationState::Completed);
        }
        let result = mutations.terminal().await;
        mutations.controller.shutdown();
        h.close().await;
        assert_eq!(
            result.state,
            OperationState::Failed,
            "{outcome}: {result:?}"
        );
        assert!(
            result.message.contains("confirm the operation again"),
            "{result:?}"
        );
        assert_eq!(
            fs::read(&source).unwrap(),
            b"keep the original",
            "{outcome}"
        );
    }
}

#[tokio::test]
async fn mutation_does_not_wait_for_copies_reserved_after_confirmation() {
    let temp = fixture();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::write(&source, b"verified first").unwrap();
    fs::create_dir(&target).unwrap();
    let mut h = Harness::native();
    let earlier = h.controller.reserve().unwrap();
    let mut mutations = Mutations::new(&mut h, &source);
    mutations.confirm().await;
    let later = h.controller.reserve().unwrap();
    h.controller
        .enqueue_reserved(request(1, std::slice::from_ref(&source), &target), earlier)
        .await;
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Completed);
    let result = mutations.terminal().await;
    drop(later);
    mutations.controller.shutdown();
    h.close().await;
    assert_eq!(result.state, OperationState::Completed);
    assert_eq!(fs::read(target.join("source")).unwrap(), b"verified first");
    assert!(!source.exists());
}

#[tokio::test]
async fn cancelling_or_shutting_down_a_predecessor_wait_never_holds_execution_capacity() {
    for shutdown in [false, true] {
        let temp = fixture();
        let source = temp.path().join("source");
        fs::write(&source, b"retained").unwrap();
        let mut h = Harness::native();
        let reservation = h.controller.reserve().unwrap();
        let mut mutations = Mutations::new(&mut h, &source);
        mutations.confirm().await;
        assert_eq!(h.mutation.available_permits(), 1);
        assert_eq!(h.io.available_permits(), 1);
        if shutdown {
            mutations.controller.shutdown();
        } else {
            mutations.controller.cancel(mutations.request.id);
        }
        let result = mutations.terminal().await;
        assert_eq!(result.state, OperationState::Cancelled);
        assert_eq!(fs::read(&source).unwrap(), b"retained");
        drop(reservation);
        mutations.controller.shutdown();
        h.close().await;
    }
}

#[tokio::test]
async fn claiming_an_earlier_native_slot_after_a_later_copy_preserves_native_first_order() {
    let temp = fixture();
    let earlier_dir = temp.path().join("earlier");
    let later_dir = temp.path().join("later");
    let target = temp.path().join("target");
    for directory in [&earlier_dir, &later_dir, &target] {
        fs::create_dir(directory).unwrap();
    }
    let earlier = earlier_dir.join("same name");
    let later = later_dir.join("same name");
    fs::write(&earlier, b"native first").unwrap();
    fs::write(&later, b"later copy").unwrap();
    let mut h = Harness::native();
    let reservation = h.controller.reserve().unwrap();
    h.controller.enqueue(request(1, &[later], &target)).await;
    h.controller
        .enqueue_reserved(request(2, &[earlier], &target), reservation)
        .await;
    assert_eq!(h.terminal(2).await.state, CopyOperationState::Completed);
    loop {
        if let CopyEvent::Conflict(conflict) = h.next().await {
            assert_eq!(conflict.id, OperationId(1));
            h.controller
                .decide(CopyDecision {
                    id: conflict.id,
                    version: conflict.version,
                    choice: CopyChoice::Skip,
                })
                .await;
            break;
        }
    }
    assert_eq!(
        h.terminal(1).await.state,
        CopyOperationState::CompletedWithErrors
    );
    h.close().await;
    assert_eq!(fs::read(target.join("same name")).unwrap(), b"native first");
}

#[tokio::test]
async fn copy_shutdown_fails_native_reservations_even_before_ui_claims_them() {
    let temp = fixture();
    let source = temp.path().join("source");
    fs::write(&source, b"retained").unwrap();
    let mut h = Harness::native();
    let reservation = h.controller.reserve().unwrap();
    let mut mutations = Mutations::new(&mut h, &source);
    mutations.confirm().await;
    h.controller.shutdown();
    let result = mutations.terminal().await;
    assert_eq!(result.state, OperationState::Failed);
    assert!(result.message.contains("confirm the operation again"));
    assert_eq!(fs::read(&source).unwrap(), b"retained");
    drop(reservation);
    mutations.controller.shutdown();
    h.close().await;
}

#[tokio::test]
async fn stopped_copy_consumer_rejects_payload_and_resolves_its_receipt() {
    let mut h = Harness::native();
    let reservation = h.controller.reserve().unwrap();
    let fence = h.controller.admission().fence().snapshot();
    // Aborting before the actor's first poll must close reserved slots too.
    h.tasks.abort_all();
    while h.tasks.join_next().await.is_some() {}
    let root = std::env::current_dir().unwrap();
    h.controller
        .enqueue_reserved(request(1, &[root.join("source")], &root), reservation)
        .await;
    let result = h.terminal(1).await;
    assert_eq!(result.state, CopyOperationState::Failed);
    assert_eq!(result.errors[0].kind, std::io::ErrorKind::BrokenPipe);
    assert!(
        tokio::time::timeout(
            Duration::from_secs(4),
            fence.wait(&Cancellation::default(), &root)
        )
        .await
        .unwrap()
        .is_err()
    );
    assert_eq!(h.controller.capacity().available_permits(), MAX_JOBS);
    assert!(h.controller.reserve().is_err());
    h.close().await;
}

struct PanickingTransfer;
impl TransferService for PanickingTransfer {
    fn prepare_transfer_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        panic!("controlled native source failure");
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        midnight_platform::LocalFileSystem::new(None).prepare_transfer_destination(path, cancel)
    }
}

#[tokio::test]
async fn native_copy_panic_fails_the_predecessor_without_deleting_the_original() {
    let temp = fixture();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::write(&source, b"retained").unwrap();
    fs::create_dir(&target).unwrap();
    let mut h = Harness::new(Some(Arc::new(PanickingTransfer)));
    let reservation = h.controller.reserve().unwrap();
    let mut mutations = Mutations::new(&mut h, &source);
    mutations.confirm().await;
    h.controller
        .enqueue_reserved(
            request(1, std::slice::from_ref(&source), &target),
            reservation,
        )
        .await;
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Failed);
    assert_eq!(mutations.terminal().await.state, OperationState::Failed);
    assert_eq!(fs::read(&source).unwrap(), b"retained");
    mutations.controller.shutdown();
    h.close().await;
}

#[tokio::test]
async fn abandoning_many_native_slots_preserves_the_bounded_capacity_and_next_copy() {
    let temp = fixture();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::write(&source, b"copied").unwrap();
    fs::create_dir(&target).unwrap();
    let mut h = Harness::native();
    let first = h.controller.reserve().unwrap();
    for _ in 0..MAX_JOBS * 8 {
        drop(h.controller.reserve().unwrap());
    }
    assert_eq!(h.controller.capacity().available_permits(), MAX_JOBS - 1);
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &target))
        .await;
    drop(first);
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Completed);
    assert_eq!(h.controller.capacity().available_permits(), MAX_JOBS);
    h.close().await;
    assert_eq!(fs::read(target.join("source")).unwrap(), b"copied");
}
