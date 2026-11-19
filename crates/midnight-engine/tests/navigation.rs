use midnight_core::{
    Cancellation, Command, DirectoryService, Entry, EntryId, EntryKind, EntryMetadata, Event,
    FsError, NavigationId, PaneId, SortSpec, SpaceInfo, StorageService, Volume, VolumeService,
};
use midnight_engine::Engine;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use tokio::sync::{Notify, mpsc};

/// Deadline of every wait for Engine progress: a listing worker entering the provider,
/// the next event, or a retired Engine cancelling its listing. Each wait ends as soon as
/// that progress happens, so the deadline elapses only in a failing test; it is long
/// because a worker can wait seconds for a CPU while other test suites saturate the
/// machine (3 s expired once that way, TEST-03).
const PROGRESS: Duration = Duration::from_secs(30);

struct ControlledFileSystem {
    entered: Arc<Notify>,
    released: Arc<(Mutex<bool>, Condvar)>,
    cooperative: bool,
    cancellation: Mutex<Option<Cancellation>>,
}

struct ReleaseOnDrop(Arc<(Mutex<bool>, Condvar)>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        let (lock, condition) = &*self.0;
        *lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        condition.notify_all();
    }
}

impl DirectoryService for ControlledFileSystem {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if path == Path::new("missing") {
            return Err(FsError::from_io(
                "read directory",
                path,
                io::Error::from(io::ErrorKind::NotFound),
            ));
        }
        if path == Path::new("slow") {
            *self.cancellation.lock().unwrap() = Some(cancel.clone());
            self.entered.notify_one();
            let (lock, condition) = &*self.released;
            let mut released = lock.lock().unwrap();
            while !*released && (!self.cooperative || !cancel.is_cancelled()) {
                released = condition
                    .wait_timeout(released, Duration::from_millis(10))
                    .unwrap()
                    .0;
            }
        }
        for index in 0..300 {
            let mut entry = Entry::new(
                EntryId(index),
                path.join(format!("file-{index:03}")),
                EntryKind::File,
            );
            if path == Path::new("cached") {
                entry.metadata.size = Some(7);
            }
            if cancel.is_cancelled() || !emit(entry) {
                break;
            }
        }
        Ok(())
    }

    fn listing_metadata_complete(&self, entry: &Entry) -> bool {
        entry.path.starts_with("cached")
    }

    fn entry_metadata(&self, _path: &Path) -> Result<EntryMetadata, FsError> {
        assert!(
            !_path.starts_with("cached"),
            "cached listing must not re-read metadata"
        );
        Ok(EntryMetadata {
            size: Some(7),
            modified: None,
            ..EntryMetadata::default()
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn complete_enumeration_metadata_is_published_without_another_read() {
    let (sender, mut events) = mpsc::channel(16);
    let mut command_engine_1 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        provider(),
        sender,
    );
    let (commands, receiver) = command_engine_1.command_channel(8).unwrap();
    let task = tokio::spawn(command_engine_1.run(receiver));
    commands
        .send(navigate(PaneId::Left, 1, "cached"))
        .await
        .unwrap();
    loop {
        match next(&mut events).await {
            Event::MetadataBatch { .. } => panic!("no redundant metadata batch"),
            Event::ListingFinished { entries, .. } => {
                assert_eq!(entries.len(), 300);
                assert!(entries.iter().all(|entry| entry.metadata.size == Some(7)));
                break;
            }
            Event::ListingFailed { error, .. } => panic!("{error}"),
            _ => {}
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

impl VolumeService for ControlledFileSystem {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![Volume {
            id: "root".into(),
            label: "Root".into(),
            root: PathBuf::from("/"),
        }])
    }
}

impl StorageService for ControlledFileSystem {
    fn storage_kind(&self, path: &Path) -> Result<midnight_core::StorageKind, FsError> {
        if path == Path::new("network") {
            Ok(midnight_core::StorageKind::Network)
        } else {
            Err(FsError::from_io(
                "storage profile",
                path,
                io::ErrorKind::Unsupported.into(),
            ))
        }
    }
    fn space_info(&self, _path: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            available: 40,
            total: 100,
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn path_storage_kind_is_reported_with_navigation_identity_and_unknown_on_failure() {
    let (sender, mut events) = mpsc::channel(16);
    let mut command_engine_2 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        provider(),
        sender,
    );
    let (commands, receiver) = command_engine_2.command_channel(8).unwrap();
    let task = tokio::spawn(command_engine_2.run(receiver));
    for (generation, path, expected) in [
        (1, "network", midnight_core::StorageKind::Network),
        (2, "unknown", midnight_core::StorageKind::Unknown),
    ] {
        commands
            .send(navigate(PaneId::Left, generation, path))
            .await
            .unwrap();
        loop {
            if let Event::StorageKindUpdated {
                pane,
                navigation,
                kind,
            } = next(&mut events).await
                && navigation == NavigationId(generation)
            {
                assert_eq!(pane, PaneId::Left);
                assert_eq!(kind, expected);
                break;
            }
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

fn provider() -> Arc<ControlledFileSystem> {
    Arc::new(ControlledFileSystem {
        entered: Arc::new(Notify::new()),
        released: Arc::new((Mutex::new(false), Condvar::new())),
        cooperative: true,
        cancellation: Mutex::new(None),
    })
}

fn navigate(pane: PaneId, generation: u64, path: &str) -> Command {
    Command::Navigate {
        pane,
        navigation: NavigationId(generation),
        path: PathBuf::from(path),
        sort: SortSpec::default(),
    }
}

async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(PROGRESS, events.recv())
        .await
        .expect("engine must deliver an event")
        .expect("engine output must remain connected")
}

/// Whether a listing enters the provider within `limit`, reading the Engine's
/// events meanwhile.
///
/// The Engine fills the test's bounded event channel without being asked: when
/// it starts (fallback icons, network and archive availability) and for each
/// navigation (listing start, storage kind, free space). A listing task sends
/// its start event before it reaches the provider, so a wait that does not read
/// the channel stalls for good once the channel is full. The events read here
/// are dropped: in this file's waits every earlier listing is held in the
/// provider, so no listing can finish or fail meanwhile.
async fn entered_within(
    entered: &Notify,
    events: &mut mpsc::Receiver<Event>,
    limit: Duration,
) -> bool {
    let entered = entered.notified();
    tokio::pin!(entered);
    tokio::time::timeout(limit, async {
        loop {
            tokio::select! {
                () = &mut entered => return,
                event = events.recv() => no_listing_outcome(event),
            }
        }
    })
    .await
    .is_ok()
}

fn no_listing_outcome(event: Option<Event>) {
    match event.expect("engine output must remain connected") {
        Event::ListingFinished {
            pane, navigation, ..
        } => panic!("{pane:?} listing {navigation:?} finished while a listing was held"),
        Event::ListingFailed {
            pane,
            navigation,
            error,
        } => panic!("{pane:?} listing {navigation:?} failed while a listing was held: {error}"),
        _ => {}
    }
}

async fn until_entered(entered: &Notify, events: &mut mpsc::Receiver<Event>) {
    assert!(
        entered_within(entered, events, Duration::from_secs(3)).await,
        "listing worker did not enter the provider within 3 s"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_left_listing_does_not_prevent_right_completion() {
    let filesystem = provider();
    let _release = ReleaseOnDrop(filesystem.released.clone());

    let (sender, mut events) = mpsc::channel(8);
    let mut command_engine_3 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        filesystem.clone(),
        sender,
    );
    let (commands, receiver) = command_engine_3.command_channel(8).unwrap();
    let task = tokio::spawn(command_engine_3.run(receiver));
    commands
        .send(navigate(PaneId::Left, 1, "slow"))
        .await
        .unwrap();
    // Eight slots: fewer than the startup and left-pane events that can precede
    // the left listing's provider call, so this wait must read them.
    until_entered(&filesystem.entered, &mut events).await;
    commands
        .send(navigate(PaneId::Right, 1, "fast"))
        .await
        .unwrap();
    loop {
        if let Event::ListingFinished {
            pane: PaneId::Right,
            entries,
            ..
        } = next(&mut events).await
        {
            assert_eq!(entries.len(), 300);
            assert_eq!(entries[0].metadata.size, Some(7));
            break;
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacing_navigation_cancels_old_completion() {
    let filesystem = provider();
    let _release = ReleaseOnDrop(filesystem.released.clone());

    let (sender, mut events) = mpsc::channel(32);
    let mut command_engine_4 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        filesystem.clone(),
        sender,
    );
    let (commands, receiver) = command_engine_4.command_channel(8).unwrap();
    let task = tokio::spawn(command_engine_4.run(receiver));
    commands
        .send(navigate(PaneId::Left, 1, "slow"))
        .await
        .unwrap();
    until_entered(&filesystem.entered, &mut events).await;
    commands
        .send(navigate(PaneId::Left, 2, "fast"))
        .await
        .unwrap();
    loop {
        match next(&mut events).await {
            Event::ListingFinished {
                navigation: NavigationId(1),
                ..
            } => panic!("cancelled listing completed"),
            Event::ListingFinished {
                navigation: NavigationId(2),
                entries,
                ..
            } => {
                assert_eq!(entries.len(), 300);
                break;
            }
            _ => {}
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn saturated_result_queue_preserves_batches_and_terminal_state() {
    let (sender, mut events) = mpsc::channel(1);
    let mut command_engine_5 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        provider(),
        sender,
    );
    let (commands, receiver) = command_engine_5.command_channel(4).unwrap();
    let task = tokio::spawn(command_engine_5.run(receiver));
    commands
        .send(navigate(PaneId::Left, 1, "fast"))
        .await
        .unwrap();
    let mut count = 0;
    loop {
        match next(&mut events).await {
            Event::EntriesBatch { entries, .. } => {
                assert!(entries.len() <= 128);
                count += entries.len();
            }
            Event::MetadataBatch { entries, .. } => assert!(entries.len() <= 128),
            Event::ListingFinished { entries, .. } => {
                assert_eq!(entries.len(), 300);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(count, 300);
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn unavailable_path_reports_failure_and_other_pane_still_works() {
    let (sender, mut events) = mpsc::channel(8);
    let mut command_engine_6 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        provider(),
        sender,
    );
    let (commands, receiver) = command_engine_6.command_channel(4).unwrap();
    let task = tokio::spawn(command_engine_6.run(receiver));
    commands
        .send(navigate(PaneId::Left, 1, "missing"))
        .await
        .unwrap();
    commands
        .send(navigate(PaneId::Right, 1, "fast"))
        .await
        .unwrap();
    let mut failure = false;
    let mut completed = false;
    while !failure || !completed {
        match next(&mut events).await {
            Event::ListingFailed {
                pane: PaneId::Left,
                error,
                ..
            } => {
                assert_eq!(error.kind, io::ErrorKind::NotFound);
                failure = true;
            }
            Event::ListingFinished {
                pane: PaneId::Right,
                ..
            } => completed = true,
            _ => {}
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn uninterruptible_old_reads_cannot_exhaust_the_other_panes_workers() {
    let filesystem = Arc::new(ControlledFileSystem {
        entered: Arc::new(Notify::new()),
        released: Arc::new((Mutex::new(false), Condvar::new())),
        cooperative: false,
        cancellation: Mutex::new(None),
    });
    let _release = ReleaseOnDrop(filesystem.released.clone());

    let (sender, mut events) = mpsc::channel(32);
    let mut command_engine_7 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        filesystem.clone(),
        sender,
    );
    let (commands, receiver) = command_engine_7.command_channel(8).unwrap();
    let task = tokio::spawn(command_engine_7.run(receiver));
    for generation in [1, 2] {
        commands
            .send(navigate(PaneId::Left, generation, "slow"))
            .await
            .unwrap();
        until_entered(&filesystem.entered, &mut events).await;
    }
    commands
        .send(navigate(PaneId::Left, 3, "slow"))
        .await
        .unwrap();
    // The third listing sends its start event right before it waits for one of
    // the pane's two listing permits, which both held reads keep.
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match next(&mut events).await {
                Event::ListingStarted {
                    pane: PaneId::Left,
                    navigation: NavigationId(3),
                    ..
                } => break,
                event => no_listing_outcome(Some(event)),
            }
        }
    })
    .await
    .expect("third listing started");
    assert!(
        !entered_within(&filesystem.entered, &mut events, Duration::from_millis(100)).await,
        "one pane must not start a third uncancellable OS read"
    );
    commands
        .send(navigate(PaneId::Right, 1, "fast"))
        .await
        .unwrap();
    loop {
        if matches!(
            next(&mut events).await,
            Event::ListingFinished {
                pane: PaneId::Right,
                ..
            }
        ) {
            break;
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn aborting_engine_cancels_the_cooperative_listing_it_owns() {
    let filesystem = provider();
    let _release = ReleaseOnDrop(filesystem.released.clone());

    let (sender, mut events) = mpsc::channel(16);
    let mut command_engine_8 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        filesystem.clone(),
        sender,
    );
    let (commands, receiver) = command_engine_8.command_channel(4).unwrap();
    let engine = tokio::spawn(command_engine_8.run(receiver));
    commands
        .send(navigate(PaneId::Left, 1, "slow"))
        .await
        .unwrap();
    until_entered(&filesystem.entered, &mut events).await;
    let cancellation = filesystem.cancellation.lock().unwrap().clone().unwrap();
    engine.abort();
    assert!(engine.await.unwrap_err().is_cancelled());
    drop(events);
    // EngineRun retires the suspended future on its dedicated worker.
    tokio::time::timeout(PROGRESS, async {
        while !cancellation.is_cancelled() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("retired engine must cancel its listing");
    assert!(
        cancellation.is_cancelled(),
        "Engine drop left its native listing uncancelled"
    );
}
