use super::*;
use crate::{CommittedSnapshot, SnapshotState};
use tokio::sync::Semaphore;

#[path = "drag_drop_ownership_tests.rs"]
mod ownership;

#[path = "drag_drop_refusal_tests.rs"]
mod refusal;

struct Harness {
    sink: Sink,
    events: mpsc::Receiver<DragDropEvent>,
    request: FileDropRequest,
}
impl Harness {
    fn new() -> Self {
        let path = std::env::current_dir().unwrap();
        let target = FileDropTarget {
            pane: PaneId::Right,
            navigation: NavigationId(5),
            directory: Arc::new(path.clone()),
            rect: DropRect {
                x: 10,
                y: 0,
                width: 100,
                height: 100,
            },
        };
        let snapshots = crate::test_snapshots([
            SnapshotState::default(),
            SnapshotState {
                archive: None,
                network: None,
                network_navigation: None,
                archive_navigation: None,
                committed: Some(CommittedSnapshot {
                    navigation: NavigationId(5),
                    path: path.clone(),
                    entries: Arc::from([]),
                    sizes: Arc::default(),
                }),
                ..Default::default()
            },
        ]);
        let (events, receiver) = mpsc::channel(64);
        let (hover, refused) = reports();
        Self {
            sink: Sink {
                host: DragHost {
                    generation: 3,
                    native: NativeDragWindow::Unsupported,
                    lifetime: Cancellation::default(),
                },
                targets: Arc::new(Mutex::new(DropTargets {
                    host_generation: 3,
                    revision: 8,
                    targets: vec![target.clone()],
                })),
                snapshots,
                pending: Arc::new(
                    crate::engine_lifetime::DragRetirement::testing()
                        .pending
                        .own(Mutex::new(HashMap::new())),
                ),
                capacity: copy_operations::Admission::default(),
                events,
                next: Arc::new(AtomicU64::new(0)),
                hover,
                refused,
                cancel: Cancellation::default(),
            },
            events: receiver,
            request: FileDropRequest {
                files: FileClipboard {
                    paths: vec![path.join("raw source 文档")],
                    intent: FileClipboardIntent::Copy,
                },
                target,
                targets_revision: 8,
            },
        }
    }
}

async fn capacity_returned(capacity: Arc<Semaphore>, expected: u32) {
    // Sealed reservations release their actual job slots only after worker retirement returns.
    let returned = tokio::time::timeout(
        Duration::from_secs(3),
        capacity.clone().acquire_many_owned(expected),
    )
    .await
    .expect("native drop retirement must return the reserved capacity")
    .expect("the fixture admission remains open");
    drop(returned);
    assert_eq!(capacity.available_permits(), expected as usize);
}

#[tokio::test]
async fn native_ack_requires_actual_capacity_and_delivery_admission() {
    let mut h = Harness::new();
    for _ in 0..32 {
        h.sink.try_accept(h.request.clone()).unwrap();
    }
    assert_eq!(h.sink.capacity.capacity().available_permits(), 0);
    assert_eq!(h.sink.pending.lock().unwrap().len(), 32);
    assert_eq!(
        h.sink.try_accept(h.request.clone()).unwrap_err().kind,
        io::ErrorKind::WouldBlock
    );
    let DragDropEvent::DropAccepted {
        token,
        item_count,
        destination,
        ..
    } = h.events.try_recv().unwrap()
    else {
        panic!("expected accepted drop")
    };
    assert_eq!(item_count, 1);
    assert_eq!(destination, h.request.target.directory);
    let captured = h.sink.pending.lock().unwrap().remove(&token).unwrap();
    assert_eq!(captured.files.paths, h.request.files.paths);
    drop(captured);
    capacity_returned(h.sink.capacity.capacity(), 1).await;

    let h = Harness::new();
    drop(h.events);
    assert!(h.sink.try_accept(h.request).is_err());
    capacity_returned(h.sink.capacity.capacity(), 32).await;
    assert!(h.sink.pending.lock().unwrap().is_empty());
}

#[test]
fn retired_owners_wake_the_preparation_worker_that_drops_them() {
    let (wake, woken) = std::sync::mpsc::sync_channel(1);
    let mut retired = Retired::default();
    // Owners retired before the worker runs need no wake: it drops them when it starts.
    retired.push(Box::new(()));
    retired.wake = Some(wake);
    retired.push(Box::new(()));
    retired.push(Box::new(()));
    assert!(
        woken.try_recv().is_ok(),
        "a retired owner must wake the worker"
    );
    assert!(woken.try_recv().is_err(), "wakes coalesce");
    assert_eq!(retired.take().len(), 3);
    retired.push(Box::new(()));
    assert!(
        woken.try_recv().is_ok(),
        "taking the owners keeps the worker's signal"
    );
}

#[test]
fn stale_regions_navigation_and_retired_hosts_cannot_accept_drop() {
    let h = Harness::new();
    let mut stale = h.request.clone();
    stale.targets_revision -= 1;
    assert!(h.sink.try_accept(stale).is_err());
    h.sink.snapshots.lock().unwrap()[1].pending = Some(NavigationId(6));
    assert!(h.sink.try_accept(h.request.clone()).is_err());
    h.sink.snapshots.lock().unwrap()[1].pending = None;
    h.sink.try_accept(h.request.clone()).unwrap();
    h.sink.host.lifetime.cancel();
    assert!(h.sink.try_accept(h.request).is_err());
    assert_eq!(
        h.sink.pending.lock().unwrap().len(),
        1,
        "already accepted destinations remain pinned"
    );
}

#[tokio::test]
async fn shutdown_winning_the_admission_lock_cannot_be_followed_by_native_copy_ack() {
    let fixture = Harness::new();
    let sink = Arc::new(fixture.sink);
    let pending = sink.pending.clone();
    let result = {
        let locked = pending.lock().unwrap();
        let worker_sink = sink.clone();
        let request = fixture.request;
        let worker = std::thread::spawn(move || worker_sink.try_accept(request));
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while sink.capacity.capacity().available_permits() == 32
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            sink.capacity.capacity().available_permits(),
            31,
            "native callback reached the admission boundary"
        );
        sink.cancel.cancel();
        drop(locked);
        worker.join().unwrap()
    };
    assert!(
        result.is_err(),
        "shutdown must win before native Copy acknowledgement"
    );
    assert!(pending.lock().unwrap().is_empty());
    capacity_returned(sink.capacity.capacity(), 32).await;
}

#[test]
fn drag_preparation_preserves_captured_native_paths_and_rejects_stale_or_hidden_fallback() {
    let h = Harness::new();
    let source = h.request.files.paths[0].clone();
    let entries: Arc<[Entry]> =
        vec![Entry::new(EntryId(1), source.clone(), EntryKind::File)].into();
    h.sink.snapshots.lock().unwrap()[1]
        .committed
        .as_mut()
        .unwrap()
        .entries = entries.clone();
    let files = SelectedFiles {
        selection: Arc::new(SelectionSnapshot::default()),
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        fallback: Some(source.clone()),
    };
    assert_eq!(
        resolve_source(
            &h.sink.snapshots,
            PaneId::Right,
            NavigationId(5),
            &files,
            &Cancellation::default()
        )
        .unwrap()
        .paths,
        [source]
    );
    assert!(
        resolve_source(
            &h.sink.snapshots,
            PaneId::Right,
            NavigationId(4),
            &files,
            &Cancellation::default()
        )
        .is_err()
    );
    let hidden = SelectedFiles {
        view: Arc::default(),
        ..files
    };
    assert!(
        resolve_source(
            &h.sink.snapshots,
            PaneId::Right,
            NavigationId(5),
            &hidden,
            &Cancellation::default()
        )
        .is_err()
    );
}

struct FailsWhilePumping;
impl FileDragDropService for FailsWhilePumping {
    fn open_session(
        &self,
        _: DragHost,
        _: SharedDropTargets,
        _: Arc<dyn FileDropSink>,
    ) -> Result<Box<dyn FileDragDropSession>, FsError> {
        Ok(Box::new(FailsWhilePumping))
    }
}
impl FileDragDropSession for FailsWhilePumping {
    fn service_pending(&mut self) -> Result<(), FsError> {
        Err(error(io::ErrorKind::BrokenPipe, "native connection lost"))
    }
    fn start_drag(&mut self, _: &FileClipboard, _: &Cancellation) -> Result<DragFinish, FsError> {
        Ok(DragFinish::Unavailable)
    }
}

#[tokio::test]
async fn partial_drag_admission_rejects_initial_and_later_hosts_without_registration() {
    use midnight_work_budget::resident::Owner;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct Native(AtomicUsize);
    impl FileDragDropService for Native {
        fn open_session(
            &self,
            _: DragHost,
            _: SharedDropTargets,
            _: Arc<dyn FileDropSink>,
        ) -> Result<Box<dyn FileDragDropSession>, FsError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FailsWhilePumping))
        }
    }

    let fixture = Harness::new();
    let native = Arc::new(Native::default());
    let (owner, resident) = Owner::start("drag-partial-test", 2).unwrap();
    let transient = tokio::runtime::Handle::current();
    let holders = crate::engine_resident_tests::hold_workers(&resident, &transient, 2);
    // One of two queue slots stays available. The native actor can queue
    // while the preparation actor's admission is refused.
    assert!(resident.spawn(&transient, Box::new(|| {})).is_ok());
    let (sender, mut events) = mpsc::channel(8);
    let mut forwarders = JoinSet::new();
    let mut tasks = ResidentTasks::default();
    let mut controller = Controller::new(
        resident,
        DragRetirement::testing(),
        Services {
            scope: TaskLifetime::testing(native.clone()),
            native: Some(native.clone()),
            transfers: None,
        },
        fixture.sink.snapshots.clone(),
        fixture.sink.capacity.clone(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut forwarders,
    );
    for generation in [3, 4] {
        let lifetime = Cancellation::default();
        let targets = fixture.sink.targets.clone();
        targets.lock().unwrap().host_generation = generation;
        let host = DragHost {
            generation,
            native: NativeDragWindow::Windows { hwnd: 1 },
            lifetime: lifetime.clone(),
        };
        controller.attach(host, targets, &mut tasks).await;
        assert!(lifetime.is_cancelled());
        assert!(matches!(
            events.try_recv().unwrap(),
            Event::DragDrop(DragDropEvent::Failed { id: None, error })
                if error.kind == io::ErrorKind::WouldBlock
        ));
        assert!(matches!(
            events.try_recv().unwrap(),
            Event::DragDrop(DragDropEvent::Ready { host_generation, supported: false })
                if host_generation == generation
        ));
        assert!(controller.state.lock().unwrap().host.is_none());
    }
    drop(holders);
    tokio::time::timeout(Duration::from_secs(5), async {
        while tasks.join_next().await.is_some() {}
        while forwarders.join_next().await.is_some() {}
    })
    .await
    .unwrap();
    assert_eq!(native.0.load(Ordering::SeqCst), 0);
    assert!(events.try_recv().is_err());
    drop(controller);
    owner.shutdown(&transient).await.unwrap();
}

#[tokio::test]
async fn native_runtime_failure_revokes_ready_before_another_gesture() {
    let fixture = Harness::new();
    let (sender, mut events) = mpsc::channel(16);
    let mut tasks = JoinSet::new();
    let mut resident_tasks = ResidentTasks::default();
    let (_owner, mut controller) = Controller::testing(
        Some(Arc::new(FailsWhilePumping)),
        None,
        fixture.sink.snapshots.clone(),
        copy_operations::Admission::default(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let mut host = fixture.sink.host.clone();
    host.native = NativeDragWindow::Windows { hwnd: 1 };
    controller
        .attach(host, fixture.sink.targets.clone(), &mut resident_tasks)
        .await;
    let revoked = tokio::time::timeout(Duration::from_secs(1), async {
        let mut was_ready = false;
        loop {
            match events.recv().await {
                Some(Event::DragDrop(DragDropEvent::Ready {
                    supported: true, ..
                })) => was_ready = true,
                Some(Event::DragDrop(DragDropEvent::Ready {
                    supported: false, ..
                })) if was_ready => return,
                None => panic!("drag event stream ended"),
                _ => {}
            }
        }
    })
    .await;
    controller.shutdown();
    while tasks.join_next().await.is_some() {}
    while resident_tasks.join_next().await.is_some() {}
    assert!(
        revoked.is_ok(),
        "a failed native session must revoke the UI's ready state"
    );
}

#[tokio::test]
async fn accepted_internal_drop_keeps_destination_and_uses_verified_copy_exactly_once() {
    accepted_drop(
        false,
        InternalDropIntent::Copy,
        TransferIntent::Copy,
        TransferVerification::FullSha256,
        false,
    )
    .await;
}

#[tokio::test]
async fn same_panel_folder_move_uses_native_queue_and_frozen_sample_policy() {
    accepted_drop(
        true,
        InternalDropIntent::Move,
        TransferIntent::Move,
        TransferVerification::SizeAndSamples,
        false,
    )
    .await;
}

#[tokio::test]
async fn same_panel_folder_control_copy_keeps_source_and_frozen_sample_policy() {
    accepted_drop(
        true,
        InternalDropIntent::Copy,
        TransferIntent::Copy,
        TransferVerification::SizeAndSamples,
        false,
    )
    .await;
}

#[tokio::test]
async fn automatic_same_volume_folder_drop_moves_in_either_panel() {
    for same_panel in [true, false] {
        accepted_drop(
            same_panel,
            InternalDropIntent::Auto,
            TransferIntent::Move,
            TransferVerification::FullSha256,
            false,
        )
        .await;
    }
}

#[derive(Debug)]
struct OccurrenceSource {
    files: FileClipboard,
    occurrence: TreeCopyOccurrence,
}
impl CapturedClipboardFiles for OccurrenceSource {
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        // FileClipboard and TreeCopyOccurrence contain only owned metadata;
        // this fixture has no native or deferred cleanup owner behind the Arc.
        if let Some(owner) = Arc::into_inner(self) {
            drop(owner);
        }
        Ok(())
    }
    fn intent(&self) -> FileClipboardIntent {
        FileClipboardIntent::Copy
    }
    fn resolve(&self, _: &Cancellation) -> Result<FileClipboard, FsError> {
        Ok(self.files.clone())
    }
    fn tree_copy_occurrence(&self, root: usize) -> Option<TreeCopyOccurrence> {
        (root == 0).then_some(self.occurrence)
    }
}

#[tokio::test]
async fn captured_nested_drop_reports_exact_tree_root_completion_for_copy_and_move() {
    for (intent, expected) in [
        (InternalDropIntent::Copy, TransferIntent::Copy),
        (InternalDropIntent::Move, TransferIntent::Move),
    ] {
        accepted_drop(
            true,
            intent,
            expected,
            TransferVerification::SizeAndSamples,
            true,
        )
        .await;
    }
}

async fn accepted_drop(
    same_panel: bool,
    intent: InternalDropIntent,
    expected: TransferIntent,
    verification: TransferVerification,
    captured_tree: bool,
) {
    // Respect the test runner's native filesystem. A WSL checkout can live on
    // DrvFS, which cannot publish verified O_TMPFILE stages; TMPDIR can point
    // to the isolated POSIX _build fixture root instead.
    let temp_root = std::env::temp_dir().join("midnight-dnd/_build/tmp");
    std::fs::create_dir_all(&temp_root).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("accepted-")
        .tempdir_in(temp_root)
        .unwrap();
    let source = temp.path().join("source");
    let destination = if same_panel {
        source.join("folder")
    } else {
        temp.path().join("destination")
    };
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&destination).unwrap();
    let source_parent = if captured_tree {
        let nested = source.join("nested");
        std::fs::create_dir(&nested).unwrap();
        nested
    } else {
        source.clone()
    };
    let file = source_parent.join("文档 literal.txt");
    let bytes = vec![0x5a; 300_017];
    std::fs::write(&file, &bytes).unwrap();
    if captured_tree {
        std::fs::write(destination.join("文档 literal.txt"), b"old destination").unwrap();
    }
    let mut entries = vec![if captured_tree {
        Entry::new(EntryId(1), source_parent, EntryKind::Directory)
    } else {
        Entry::new(EntryId(1), file.clone(), EntryKind::File)
    }];
    if same_panel {
        entries.push(Entry::new(
            EntryId(2),
            destination.clone(),
            EntryKind::Directory,
        ));
    }
    let entries: Arc<[Entry]> = entries.into();
    let snapshots = crate::test_snapshots([
        SnapshotState {
            archive: None,
            network: None,
            network_navigation: None,
            archive_navigation: None,
            committed: Some(CommittedSnapshot {
                navigation: NavigationId(1),
                path: source,
                entries: entries.clone(),
                sizes: Arc::default(),
            }),
            ..Default::default()
        },
        SnapshotState {
            archive: None,
            network: None,
            network_navigation: None,
            archive_navigation: None,
            committed: Some(CommittedSnapshot {
                navigation: NavigationId(2),
                path: destination.clone(),
                entries: Arc::from([]),
                sizes: Arc::default(),
            }),
            ..Default::default()
        },
    ]);
    let target = FileDropTarget {
        pane: PaneId::Right,
        navigation: NavigationId(2),
        directory: Arc::new(destination.clone()),
        rect: DropRect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        },
    };
    let targets = Arc::new(Mutex::new(DropTargets {
        host_generation: 1,
        revision: 1,
        targets: vec![target.clone()],
    }));
    let (sender, mut events) = mpsc::channel(128);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    let mut tasks = JoinSet::new();
    let service = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let copies = copy_operations::Controller::new(
        crate::worker_retirement::ActorRetirement::testing(),
        Some(crate::transfer_composition::QualifiedTransferInstallation::audited(service.clone())),
        outbox.clone(),
        Arc::new(Semaphore::new(1)),
        crate::transfer_leases::IoBudget::new(Arc::new(midnight_work_budget::Gate::new(1))),
        &mut tasks,
    );
    let mut resident_tasks = ResidentTasks::default();
    let (_owner, mut controller) = Controller::testing(
        None,
        Some(service),
        snapshots.clone(),
        copies.admission(),
        outbox,
        &mut tasks,
    );
    controller
        .attach(
            DragHost {
                generation: 1,
                native: NativeDragWindow::Unsupported,
                lifetime: Cancellation::default(),
            },
            targets.clone(),
            &mut resident_tasks,
        )
        .await;
    let occurrence = TreeCopyOccurrence {
        stamp: TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            session: TreeSessionId(4),
            revision: TreeRevision(5),
        },
        row: TreeRowId(42),
    };
    let retained_source = if captured_tree {
        let captured = Arc::new(OccurrenceSource {
            files: FileClipboard {
                paths: vec![file.clone()],
                intent: FileClipboardIntent::Copy,
            },
            occurrence,
        });
        let weak = Arc::downgrade(&captured);
        controller
            .prepare_captured(DragId(1), PaneId::Left, NavigationId(1), captured)
            .await;
        Some(weak)
    } else {
        controller
            .prepare(
                DragId(1),
                PaneId::Left,
                NavigationId(1),
                SelectedFiles {
                    selection: Arc::default(),
                    view: Arc::new(DirectoryIndex::from_entries(&entries)),
                    fallback: Some(file.clone()),
                },
            )
            .await;
        None
    };
    let prepared = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                events.recv().await,
                Some(Event::DragDrop(DragDropEvent::Prepared {
                    id: DragId(1),
                    ..
                }))
            ) {
                break;
            }
        }
    })
    .await;
    assert!(prepared.is_ok());
    controller
        .internal_request(InternalDropRequest {
            id: DragId(1),
            destination: if same_panel {
                InternalDropDestination::PanelEntry {
                    pane: PaneId::Left,
                    navigation: NavigationId(1),
                    entry: EntryId(2),
                }
            } else {
                InternalDropDestination::PaneRoot {
                    pane: PaneId::Right,
                    navigation: NavigationId(2),
                }
            },
            intent,
            verification,
        })
        .await;
    let token = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(Event::DragDrop(DragDropEvent::DropAccepted {
                token,
                intent,
                verification: captured,
                ..
            })) = events.recv().await
            {
                assert_eq!(intent, expected);
                assert_eq!(captured, Some(verification));
                break token;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(copies.capacity().available_permits(), 31);
    // Navigation after native acceptance must not retarget or cancel the queued copy.
    snapshots.lock().unwrap()[usize::from(!same_panel)].pending = Some(NavigationId(3));
    targets.lock().unwrap().targets.clear();
    controller
        .accept(token, OperationId(1), Default::default(), &copies)
        .await;
    controller
        .accept(token, OperationId(2), Default::default(), &copies)
        .await;
    let mut completed = false;
    let terminal = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let Some(Event::Copy(event)) = events.recv().await {
                if let CopyEvent::Conflict(conflict) = event {
                    // The source resolver retires its gesture owner independently;
                    // the paused native job must retain its own provider authority.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    assert!(retained_source.as_ref().unwrap().upgrade().is_some(), "prepared job released its captured provider before native handles and cleanup");
                    assert!(conflict.can_replace);
                    copies.decide(CopyDecision { id: conflict.id, version: conflict.version, choice: CopyChoice::ReplaceRecoverable { all: false } }).await;
                    continue;
                }
                if let CopyEvent::RootsCompleted { id, sources } = event {
                    assert_eq!(id, OperationId(1));
                    for source in sources {
                        if captured_tree { assert!(matches!(source, CompletedCopySource::Tree(found) if found == occurrence), "nested drop lost its exact tree occurrence: {source:?}"); }
                    }
                    completed = true;
                    continue;
                }
                let CopyEvent::Progress(progress) = event else { continue; };
                assert_eq!(
                    progress.id,
                    OperationId(1),
                    "a replayed token cannot queue another copy"
                );
                if progress.state.is_terminal() {
                    break progress;
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(completed);
    assert!(
        retained_source
            .as_ref()
            .is_none_or(|source| source.upgrade().is_none()),
        "terminal job retained its provider after native cleanup"
    );
    assert_eq!(
        terminal.state,
        CopyOperationState::Completed,
        "{:?}",
        terminal.errors
    );
    assert_eq!(terminal.verification, verification);
    assert_eq!(terminal.intent, expected);
    if expected == TransferIntent::Move {
        assert_eq!(terminal.native_entries_moved, 1);
        assert_eq!(
            terminal.copied_bytes + terminal.verified_bytes + terminal.validation_bytes,
            0
        );
        assert!(!file.exists());
    } else {
        assert_eq!(
            terminal.verified_bytes,
            if verification == TransferVerification::FullSha256 {
                bytes.len() as u64
            } else {
                196_608
            }
        );
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
    }
    assert_eq!(terminal.destination_directory, destination);
    assert_eq!(
        std::fs::read(destination.join("文档 literal.txt")).unwrap(),
        bytes
    );
    assert_eq!(copies.capacity().available_permits(), 32);
    controller.shutdown();
    copies.shutdown();
    while tasks.join_next().await.is_some() {}
    while resident_tasks.join_next().await.is_some() {}
}

#[tokio::test]
async fn shutdown_releases_unclaimed_native_drop_reservations() {
    let fixture = Harness::new();
    let (sender, _events) = mpsc::channel(128);
    let mut tasks = JoinSet::new();
    let mut resident_tasks = ResidentTasks::default();
    let (_owner, mut controller) = Controller::testing(
        None,
        None,
        fixture.sink.snapshots.clone(),
        fixture.sink.capacity.clone(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    controller
        .attach(
            fixture.sink.host.clone(),
            fixture.sink.targets.clone(),
            &mut resident_tasks,
        )
        .await;
    let sink = controller
        .state
        .lock()
        .unwrap()
        .host
        .as_ref()
        .unwrap()
        .sink
        .clone();
    sink.try_accept(fixture.request).unwrap();
    assert_eq!(fixture.sink.capacity.capacity().available_permits(), 31);
    controller.shutdown();
    while tasks.join_next().await.is_some() {}
    while resident_tasks.join_next().await.is_some() {}
    capacity_returned(fixture.sink.capacity.capacity(), 32).await;
    assert!(controller.pending.lock().unwrap().is_empty());
}

#[derive(Default)]
struct NativeObservations {
    started: std::sync::atomic::AtomicBool,
    thread: Mutex<Option<std::thread::ThreadId>>,
    dropped: std::sync::atomic::AtomicBool,
}
struct BlockingNative(Arc<NativeObservations>);
struct BlockingSession(Arc<NativeObservations>);
impl FileDragDropService for BlockingNative {
    fn open_session(
        &self,
        _: DragHost,
        _: SharedDropTargets,
        _: Arc<dyn FileDropSink>,
    ) -> Result<Box<dyn FileDragDropSession>, FsError> {
        *self.0.thread.lock().unwrap() = Some(std::thread::current().id());
        Ok(Box::new(BlockingSession(self.0.clone())))
    }
}
impl FileDragDropSession for BlockingSession {
    fn service_pending(&mut self) -> Result<(), FsError> {
        assert_eq!(
            *self.0.thread.lock().unwrap(),
            Some(std::thread::current().id())
        );
        Ok(())
    }
    fn start_drag(
        &mut self,
        _: &FileClipboard,
        cancel: &Cancellation,
    ) -> Result<DragFinish, FsError> {
        assert_eq!(
            *self.0.thread.lock().unwrap(),
            Some(std::thread::current().id())
        );
        self.0.started.store(true, Ordering::SeqCst);
        while !cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(DragFinish::Cancelled)
    }
}
impl Drop for BlockingSession {
    fn drop(&mut self) {
        assert_eq!(
            *self.0.thread.lock().unwrap(),
            Some(std::thread::current().id())
        );
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn native_drag_keeps_one_actor_thread_and_cancellation_remains_responsive() {
    let fixture = Harness::new();
    let file = fixture.request.files.paths[0].clone();
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), file.clone(), EntryKind::File)].into();
    fixture.sink.snapshots.lock().unwrap()[1]
        .committed
        .as_mut()
        .unwrap()
        .entries = entries.clone();
    let observations = Arc::new(NativeObservations::default());
    let service = Arc::new(BlockingNative(observations.clone()));
    let (sender, mut events) = mpsc::channel(32);
    let mut tasks = JoinSet::new();
    let mut resident_tasks = ResidentTasks::default();
    let (_owner, mut controller) = Controller::testing(
        Some(service.clone()),
        None,
        fixture.sink.snapshots.clone(),
        fixture.sink.capacity.clone(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let mut host = fixture.sink.host.clone();
    host.native = NativeDragWindow::Windows { hwnd: 1 };
    controller
        .attach(host, fixture.sink.targets, &mut resident_tasks)
        .await;
    controller
        .prepare(
            DragId(1),
            PaneId::Right,
            NavigationId(5),
            SelectedFiles {
                selection: Arc::default(),
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                fallback: Some(file),
            },
        )
        .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                events.recv().await,
                Some(Event::DragDrop(DragDropEvent::Prepared { .. }))
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();
    controller.start(DragId(1)).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while !observations.started.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    assert_ne!(
        *observations.thread.lock().unwrap(),
        Some(std::thread::current().id())
    );
    tokio::time::timeout(Duration::from_millis(100), controller.cancel(DragId(1)))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if matches!(
                events.recv().await,
                Some(Event::DragDrop(DragDropEvent::Finished {
                    id: DragId(1),
                    outcome: DragFinish::Cancelled
                }))
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();
    controller.shutdown();
    while tasks.join_next().await.is_some() {}
    while resident_tasks.join_next().await.is_some() {}
    assert!(observations.dropped.load(Ordering::SeqCst));
}
