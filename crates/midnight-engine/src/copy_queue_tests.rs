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
        let snapshots = crate::test_snapshots([
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
        ]);
        let (sender, events) = mpsc::channel(64);
        let controller = operations::Controller::new(
            crate::worker_retirement::ActorRetirement::testing(),
            operations::Services {
                mutation: Some(Arc::new(midnight_platform::LocalFileSystem::new(None))),
                locale: None,
            },
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

#[test]
fn legacy_native_input_conversions_keep_reject_link_policy() {
    let root = std::env::current_dir().unwrap();
    let source = root.join("selected-link");
    let paths = request(1, std::slice::from_ref(&source), &root);
    let entries = [Entry::new(EntryId(1), source, EntryKind::Symlink)];
    let selected = selected_request(1, &entries, &root);
    let clipboard = ClipboardMoveRequest {
        id: OperationId(1),
        clipboard: Arc::new(paths.clipboard.clone()),
        destination: root,
        verification: paths.verification,
    };
    for input in [
        CopyInput::from(paths.clone()),
        CopyInput::from(selected.clone()),
        CopyInput::metadata(MetadataTransferRequest {
            input: MetadataTransferInput::Paths(paths.clone()),
            intent: TransferIntent::Copy,
            metadata: TransferMetadataPolicy::ContentOnly,
        }),
        CopyInput::move_paths(paths),
        CopyInput::move_selection(selected),
        CopyInput::move_clipboard(clipboard),
    ] {
        assert_eq!(input.links, CopyLinkPolicy::Reject);
    }
}

#[tokio::test]
async fn explicit_link_options_share_the_32_job_limit_and_freeze_preserve_after_admission() {
    let fixture = super::super::link_tests::Fixture::new();
    let mut h = Harness::new(Some(fixture.service.clone()));
    let reservations: Vec<_> = (0..32).map(|_| h.controller.reserve().unwrap()).collect();
    let refused_options = fixture.options(1);
    h.controller.enqueue_options(refused_options.clone()).await;
    let refused = h.terminal(1).await;
    assert_eq!(refused.errors[0].kind, io::ErrorKind::WouldBlock);
    assert_eq!(refused_options.links, CopyLinkPolicy::Preserve);
    assert_eq!(fixture.control.source_opens.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.control.destination_opens.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 0);
    drop(reservations);
    let gate = h.mutation.clone().acquire_owned().await.unwrap();
    let mut draft = fixture.options(2);
    h.controller.enqueue_options(draft.clone()).await;
    draft.links = CopyLinkPolicy::Reject;
    drop(gate);
    let done = h.terminal(2).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.files_completed, 1);
    assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 1);
    assert_eq!(h.controller.capacity().available_permits(), MAX_JOBS);
    assert_eq!(draft.links, CopyLinkPolicy::Reject);
    h.close().await;
}

struct LinkSyncRelease(Option<std::sync::mpsc::Sender<()>>);

impl Drop for LinkSyncRelease {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

fn observe_link_resources(fixture: &super::super::link_tests::Fixture, h: &Harness) {
    fixture
        .control
        .observe_resources(h.io.clone(), h.mutation.clone(), h.controller.capacity());
}

fn assert_link_resources_held_until_drop(
    fixture: &super::super::link_tests::Fixture,
    expected_owners: &[&str],
) {
    let snapshots = fixture.control.resource_snapshots();
    let owners = snapshots
        .iter()
        .map(|(owner, ..)| *owner)
        .collect::<Vec<_>>();
    assert_eq!(owners, expected_owners);
    assert!(
        snapshots.iter().all(|(_, io, mutation, capacity)| {
            *io == 0 && *mutation == 0 && *capacity == MAX_JOBS - 1
        }),
        "cleanup and every native/provider destructor retain all leases: {snapshots:?}"
    );
}

async fn wait_for_retired_copy_capacity(h: &Harness) {
    let capacity = h.controller.capacity();
    assert!(capacity.is_closed(), "shutdown keeps Copy admission closed");
    tokio::time::timeout(Duration::from_secs(4), async {
        while capacity.available_permits() != MAX_JOBS {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("actual cleanup and retained-owner destruction must return every Copy permit");
    assert!(capacity.is_closed());
    assert_eq!(capacity.available_permits(), MAX_JOBS);
}

async fn staged_link_decision(h: &mut Harness, partial: bool) {
    loop {
        match h.next().await {
            CopyEvent::Conflict(conflict) if !partial => {
                assert!(!conflict.can_replace && !conflict.can_merge);
                return;
            }
            CopyEvent::Failure(failure) if partial => {
                assert!(!failure.can_retry);
                return;
            }
            CopyEvent::Progress(progress) => assert!(
                !progress.state.is_terminal(),
                "expected an owned staged decision: {progress:?}"
            ),
            event => panic!("unexpected staged decision event: {event:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_during_blocked_link_sync_keeps_actual_return_ownership_and_worker_drops() {
    let fixture = super::super::link_tests::Fixture::new();
    let registration = fixture.retain_registration();
    let registered = Arc::downgrade(&registration);
    let entered = Arc::new(Notify::new());
    let (release, released) = std::sync::mpsc::channel();
    let mut release = LinkSyncRelease(Some(release));
    *fixture.control.sync_block.lock().unwrap() = Some(super::super::link_tests::SyncBlock {
        entered: entered.clone(),
        release: released,
    });
    let cleanup_entered = Arc::new(Notify::new());
    let (cleanup_release, cleanup_released) = std::sync::mpsc::channel();
    let mut cleanup_release = LinkSyncRelease(Some(cleanup_release));
    *fixture.control.cleanup_block.lock().unwrap() = Some(super::super::link_tests::SyncBlock {
        entered: cleanup_entered.clone(),
        release: cleanup_released,
    });
    let mut h = Harness::new(Some(fixture.service.clone()));
    observe_link_resources(&fixture, &h);
    let dispatcher = std::thread::current().id();
    h.controller.enqueue_options(fixture.options(1)).await;
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();
    drop(registration);
    h.tasks.abort_all();
    while h.tasks.join_next().await.is_some() {}
    assert_eq!(h.io.available_permits(), 0);
    assert_eq!(h.mutation.available_permits(), 0);
    assert_eq!(h.controller.capacity().available_permits(), MAX_JOBS - 1);
    assert!(fixture.control.drop_threads().is_empty());
    assert_eq!(fixture.control.cleanups.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.control.staged_count(), 1);
    assert!(registered.upgrade().is_some());
    release.0.take().unwrap().send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(4), cleanup_entered.notified())
        .await
        .unwrap();
    assert_eq!(h.io.available_permits(), 0);
    assert_eq!(h.mutation.available_permits(), 0);
    assert_eq!(h.controller.capacity().available_permits(), MAX_JOBS - 1);
    assert!(fixture.control.owner_drops().is_empty());
    assert_eq!(fixture.control.staged_count(), 1);
    assert!(registered.upgrade().is_some());
    cleanup_release.0.take().unwrap().send(()).unwrap();
    wait_for_retired_copy_capacity(&h).await;
    let io = tokio::time::timeout(Duration::from_secs(4), h.io.clone().acquire_owned())
        .await
        .unwrap()
        .unwrap();
    let mutation = tokio::time::timeout(Duration::from_secs(4), h.mutation.clone().acquire_owned())
        .await
        .unwrap()
        .unwrap();
    let drops = fixture.control.drop_threads();
    assert!(drops.len() >= 2, "source and stage must both retire");
    assert!(drops.iter().all(|thread| *thread != dispatcher));
    assert_eq!(fixture.control.cleanups.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.control.staged_count(), 0);
    assert_eq!(
        fixture.control.owner_drops(),
        vec![("stage", true), ("source", true)]
    );
    assert!(registered.upgrade().is_none());
    assert_link_resources_held_until_drop(&fixture, &["stage", "source", "provider"]);
    drop((io, mutation));
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decision_abort_keeps_full_and_partial_link_stages_under_the_dedicated_io_permit() {
    for partial in [false, true] {
        let fixture = super::super::link_tests::Fixture::new();
        fixture
            .control
            .late_collision
            .store(!partial, Ordering::SeqCst);
        fixture
            .control
            .fail_stage_creation
            .store(partial, Ordering::SeqCst);
        let registration = fixture.retain_registration();
        let registered = Arc::downgrade(&registration);
        let cleanup_entered = Arc::new(Notify::new());
        let (release, released) = std::sync::mpsc::channel();
        let mut release = LinkSyncRelease(Some(release));
        *fixture.control.cleanup_block.lock().unwrap() =
            Some(super::super::link_tests::SyncBlock {
                entered: cleanup_entered.clone(),
                release: released,
            });
        let mut h = Harness::new(Some(fixture.service.clone()));
        observe_link_resources(&fixture, &h);
        let dispatcher = std::thread::current().id();
        h.controller.enqueue_options(fixture.options(1)).await;
        staged_link_decision(&mut h, partial).await;
        assert!(
            h.io.clone().try_acquire_owned().is_err(),
            "a staged decision retains its actual Copy permit"
        );
        assert_eq!(fixture.control.staged_count(), 1);
        drop(registration);
        h.tasks.abort_all();
        while h.tasks.join_next().await.is_some() {}
        tokio::time::timeout(Duration::from_secs(4), cleanup_entered.notified())
            .await
            .unwrap();
        assert_eq!(h.io.available_permits(), 0);
        assert_eq!(h.mutation.available_permits(), 0);
        assert_eq!(h.controller.capacity().available_permits(), MAX_JOBS - 1);
        assert!(fixture.control.owner_drops().is_empty());
        assert!(registered.upgrade().is_some());
        assert_eq!(fixture.control.staged_count(), 1);
        release.0.take().unwrap().send(()).unwrap();
        wait_for_retired_copy_capacity(&h).await;
        assert_eq!(h.io.available_permits(), 1);
        assert_eq!(h.mutation.available_permits(), 1);
        assert_eq!(fixture.control.cleanups.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.control.staged_count(), 0);
        assert_eq!(
            fixture.control.owner_drops(),
            vec![("stage", true), ("source", true)]
        );
        assert!(
            fixture
                .control
                .drop_threads()
                .iter()
                .all(|thread| *thread != dispatcher)
        );
        assert!(registered.upgrade().is_none());
        assert_link_resources_held_until_drop(&fixture, &["stage", "source", "provider"]);
        h.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multiple_link_roots_continue_and_finish_with_the_retained_copy_permit() {
    let fixture = super::super::link_tests::Fixture::with_two_roots();
    let registration = fixture.retain_registration();
    let registered = Arc::downgrade(&registration);
    let cleanup_entered = Arc::new(Notify::new());
    let (release, released) = std::sync::mpsc::channel();
    let mut release = LinkSyncRelease(Some(release));
    *fixture.control.cleanup_block.lock().unwrap() = Some(super::super::link_tests::SyncBlock {
        entered: cleanup_entered.clone(),
        release: released,
    });
    let mut h = Harness::new(Some(fixture.service.clone()));
    observe_link_resources(&fixture, &h);
    h.controller.enqueue_options(fixture.options(1)).await;
    tokio::time::timeout(Duration::from_secs(4), cleanup_entered.notified())
        .await
        .unwrap();
    drop(registration);
    assert!(h.io.clone().try_acquire_owned().is_err());
    assert_eq!(h.controller.capacity().available_permits(), MAX_JOBS - 1);
    assert!(fixture.control.owner_drops().is_empty());
    assert!(registered.upgrade().is_some());
    release.0.take().unwrap().send(()).unwrap();
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!((done.files_completed, done.completed_roots), (2, 2));
    assert_eq!(
        (done.copied_bytes, done.verified_bytes, done.completed_bytes),
        (0, 0, 0)
    );
    assert_eq!(fixture.control.stages_created.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.control.cleanups.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.control.staged_count(), 0);
    assert_eq!(h.io.available_permits(), 1);
    assert_eq!(h.controller.capacity().available_permits(), MAX_JOBS);
    assert!(registered.upgrade().is_none());
    assert_link_resources_held_until_drop(
        &fixture,
        &["stage", "source", "stage", "source", "provider"],
    );
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_staged_link_decisions_reuses_the_copy_permit_for_actual_cleanup() {
    for partial in [false, true] {
        let fixture = super::super::link_tests::Fixture::new();
        fixture
            .control
            .late_collision
            .store(!partial, Ordering::SeqCst);
        fixture
            .control
            .fail_stage_creation
            .store(partial, Ordering::SeqCst);
        let registration = fixture.retain_registration();
        let registered = Arc::downgrade(&registration);
        let cleanup_entered = Arc::new(Notify::new());
        let (release, released) = std::sync::mpsc::channel();
        let mut release = LinkSyncRelease(Some(release));
        *fixture.control.cleanup_block.lock().unwrap() =
            Some(super::super::link_tests::SyncBlock {
                entered: cleanup_entered.clone(),
                release: released,
            });
        let mut h = Harness::new(Some(fixture.service.clone()));
        observe_link_resources(&fixture, &h);
        h.controller.enqueue_options(fixture.options(1)).await;
        staged_link_decision(&mut h, partial).await;
        drop(registration);
        assert!(h.io.clone().try_acquire_owned().is_err());
        h.controller.cancel(OperationId(1));
        tokio::time::timeout(Duration::from_secs(4), cleanup_entered.notified())
            .await
            .unwrap();
        assert_eq!(h.io.available_permits(), 0);
        assert_eq!(h.controller.capacity().available_permits(), MAX_JOBS - 1);
        assert!(fixture.control.owner_drops().is_empty());
        assert!(registered.upgrade().is_some());
        release.0.take().unwrap().send(()).unwrap();
        let done = h.terminal(1).await;
        assert_eq!(done.state, CopyOperationState::Cancelled, "{done:?}");
        assert_eq!(done.files_completed, 0);
        assert_eq!(fixture.control.cleanups.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.control.staged_count(), 0);
        assert_eq!(h.io.available_permits(), 1);
        assert_eq!(h.controller.capacity().available_permits(), MAX_JOBS);
        assert!(registered.upgrade().is_none());
        assert_link_resources_held_until_drop(&fixture, &["stage", "source", "provider"]);
        h.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forced_partial_stage_drop_reports_real_cleanup_refusal_under_the_retained_permit() {
    let fixture = super::super::link_tests::Fixture::new();
    fixture
        .control
        .fail_stage_creation
        .store(true, Ordering::SeqCst);
    fixture.control.refuse_cleanup.store(true, Ordering::SeqCst);
    let registration = fixture.retain_registration();
    let registered = Arc::downgrade(&registration);
    let cleanup_entered = Arc::new(Notify::new());
    let (release, released) = std::sync::mpsc::channel();
    let mut release = LinkSyncRelease(Some(release));
    *fixture.control.cleanup_block.lock().unwrap() = Some(super::super::link_tests::SyncBlock {
        entered: cleanup_entered.clone(),
        release: released,
    });
    let mut h = Harness::new(Some(fixture.service.clone()));
    observe_link_resources(&fixture, &h);
    h.controller.enqueue_options(fixture.options(1)).await;
    staged_link_decision(&mut h, true).await;
    let status = h.controller.registry.lock().unwrap()[&OperationId(1)]
        .status
        .clone();
    drop(registration);
    h.tasks.abort_all();
    while h.tasks.join_next().await.is_some() {}
    tokio::time::timeout(Duration::from_secs(4), cleanup_entered.notified())
        .await
        .unwrap();
    assert!(h.io.clone().try_acquire_owned().is_err());
    assert_eq!(h.controller.capacity().available_permits(), MAX_JOBS - 1);
    assert!(fixture.control.owner_drops().is_empty());
    assert!(registered.upgrade().is_some());
    release.0.take().unwrap().send(()).unwrap();
    wait_for_retired_copy_capacity(&h).await;
    let retained = snapshot(&status);
    assert_eq!(fixture.control.cleanups.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.control.staged_count(), 1);
    assert_eq!(retained.recovery_count, 1);
    assert_eq!(
        retained.recovery_paths,
        vec![fixture.destination.join(".owned-link-100")]
    );
    assert!(
        retained.errors.iter().any(|error| {
            error.path.ends_with(".owned-link-100") && error.kind == io::ErrorKind::PermissionDenied
        }),
        "real cleanup refusal must survive forced retirement: {retained:?}"
    );
    assert_eq!(retained.source_files_removed, 0);
    assert_eq!(h.io.available_permits(), 1);
    assert!(registered.upgrade().is_none());
    assert_link_resources_held_until_drop(&fixture, &["stage", "source", "provider"]);
    h.close().await;
}
