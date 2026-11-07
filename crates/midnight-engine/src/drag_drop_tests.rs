use super::*;
use crate::{CommittedSnapshot, SnapshotState};

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
        let snapshots = Arc::new(Mutex::new([
            SnapshotState::default(),
            SnapshotState {
                archive: None,
                archive_navigation: None,
                committed: Some(CommittedSnapshot {
                    navigation: NavigationId(5),
                    path: path.clone(),
                    entries: Arc::from([]),
                    sizes: Arc::default(),
                }),
                ..Default::default()
            },
        ]));
        let (events, receiver) = mpsc::channel(64);
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
                pending: Arc::default(),
                capacity: Arc::new(Semaphore::new(32)),
                events,
                next: Arc::new(AtomicU64::new(0)),
                hover: Arc::default(),
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

#[test]
fn native_ack_requires_actual_capacity_and_delivery_admission() {
    let mut h = Harness::new();
    for _ in 0..32 {
        h.sink.try_accept(h.request.clone()).unwrap();
    }
    assert_eq!(h.sink.capacity.available_permits(), 0);
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
    assert_eq!(captured.request.files.paths, h.request.files.paths);
    drop(captured);
    assert_eq!(h.sink.capacity.available_permits(), 1);

    let h = Harness::new();
    drop(h.events);
    assert!(h.sink.try_accept(h.request).is_err());
    assert_eq!(h.sink.capacity.available_permits(), 32);
    assert!(h.sink.pending.lock().unwrap().is_empty());
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

#[test]
fn shutdown_winning_the_admission_lock_cannot_be_followed_by_native_copy_ack() {
    let fixture = Harness::new();
    let sink = Arc::new(fixture.sink);
    let pending = sink.pending.clone();
    let locked = pending.lock().unwrap();
    let worker_sink = sink.clone();
    let request = fixture.request;
    let worker = std::thread::spawn(move || worker_sink.try_accept(request));
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while sink.capacity.available_permits() == 32 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        sink.capacity.available_permits(),
        31,
        "native callback reached the admission boundary"
    );
    sink.cancel.cancel();
    drop(locked);
    let result = worker.join().unwrap();
    assert!(
        result.is_err(),
        "shutdown must win before native Copy acknowledgement"
    );
    assert!(pending.lock().unwrap().is_empty());
    assert_eq!(sink.capacity.available_permits(), 32);
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
async fn native_runtime_failure_revokes_ready_before_another_gesture() {
    let fixture = Harness::new();
    let (sender, mut events) = mpsc::channel(16);
    let mut tasks = JoinSet::new();
    let mut controller = Controller::new(
        Some(Arc::new(FailsWhilePumping)),
        fixture.sink.snapshots.clone(),
        Arc::new(Semaphore::new(32)),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let mut host = fixture.sink.host.clone();
    host.native = NativeDragWindow::Windows { hwnd: 1 };
    controller
        .attach(host, fixture.sink.targets.clone(), &mut tasks)
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
    assert!(
        revoked.is_ok(),
        "a failed native session must revoke the UI's ready state"
    );
}

#[tokio::test]
async fn accepted_internal_drop_keeps_destination_and_uses_verified_copy_exactly_once() {
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
    let destination = temp.path().join("destination");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&destination).unwrap();
    let file = source.join("文档 literal.txt");
    let bytes = vec![0x5a; 300_017];
    std::fs::write(&file, &bytes).unwrap();
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), file.clone(), EntryKind::File)].into();
    let snapshots = Arc::new(Mutex::new([
        SnapshotState {
            archive: None,
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
            archive_navigation: None,
            committed: Some(CommittedSnapshot {
                navigation: NavigationId(2),
                path: destination.clone(),
                entries: Arc::from([]),
                sizes: Arc::default(),
            }),
            ..Default::default()
        },
    ]));
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
    let copies = copy_operations::Controller::new(
        Some(Arc::new(midnight_platform::LocalFileSystem::new(None))),
        outbox.clone(),
        Arc::new(Semaphore::new(1)),
        Arc::new(Semaphore::new(1)),
        &mut tasks,
    );
    let mut controller = Controller::new(
        None,
        snapshots.clone(),
        copies.capacity(),
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
                fallback: Some(file.clone()),
            },
        )
        .await;
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
    controller.internal(DragId(1), target, 1).await;
    let token = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(Event::DragDrop(DragDropEvent::DropAccepted { token, .. })) =
                events.recv().await
            {
                break token;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(copies.capacity().available_permits(), 31);
    // Navigation after native acceptance must not retarget or cancel the queued copy.
    snapshots.lock().unwrap()[1].pending = Some(NavigationId(3));
    targets.lock().unwrap().targets.clear();
    controller.accept(token, OperationId(1), &copies).await;
    controller.accept(token, OperationId(2), &copies).await;
    let terminal = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let Some(Event::Copy(CopyEvent::Progress(progress))) = events.recv().await {
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
    assert_eq!(
        terminal.state,
        CopyOperationState::Completed,
        "{:?}",
        terminal.errors
    );
    assert_eq!(terminal.verified_bytes, bytes.len() as u64);
    assert_eq!(terminal.destination_directory, destination);
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    assert_eq!(
        std::fs::read(destination.join("文档 literal.txt")).unwrap(),
        bytes
    );
    assert_eq!(copies.capacity().available_permits(), 32);
    controller.shutdown();
    copies.shutdown();
    while tasks.join_next().await.is_some() {}
}

#[tokio::test]
async fn shutdown_releases_unclaimed_native_drop_reservations() {
    let fixture = Harness::new();
    let (sender, _events) = mpsc::channel(128);
    let mut tasks = JoinSet::new();
    let mut controller = Controller::new(
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
            &mut tasks,
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
    assert_eq!(fixture.sink.capacity.available_permits(), 31);
    controller.shutdown();
    assert_eq!(fixture.sink.capacity.available_permits(), 32);
    assert!(controller.pending.lock().unwrap().is_empty());
    while tasks.join_next().await.is_some() {}
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
    let mut controller = Controller::new(
        Some(service.clone()),
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
        .attach(host, fixture.sink.targets, &mut tasks)
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
    assert!(observations.dropped.load(Ordering::SeqCst));
}
