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
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            provider(),
            sender,
        )
        .run(receiver),
    );
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
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            provider(),
            sender,
        )
        .run(receiver),
    );
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
    tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .expect("engine must deliver an event")
        .expect("engine output must remain connected")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_left_listing_does_not_prevent_right_completion() {
    let filesystem = provider();
    let _release = ReleaseOnDrop(filesystem.released.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(8);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, "slow"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), filesystem.entered.notified())
        .await
        .expect("left worker entered provider");
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
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, "slow"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), filesystem.entered.notified())
        .await
        .unwrap();
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
    let (commands, receiver) = mpsc::channel(4);
    let (sender, mut events) = mpsc::channel(1);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            provider(),
            sender,
        )
        .run(receiver),
    );
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
    let (commands, receiver) = mpsc::channel(4);
    let (sender, mut events) = mpsc::channel(8);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            provider(),
            sender,
        )
        .run(receiver),
    );
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
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    for generation in [1, 2] {
        commands
            .send(navigate(PaneId::Left, generation, "slow"))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), filesystem.entered.notified())
            .await
            .unwrap();
    }
    commands
        .send(navigate(PaneId::Left, 3, "slow"))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), filesystem.entered.notified())
            .await
            .is_err(),
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
    let (commands, receiver) = mpsc::channel(4);
    let (sender, events) = mpsc::channel(16);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, "slow"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), filesystem.entered.notified())
        .await
        .unwrap();
    let cancellation = filesystem.cancellation.lock().unwrap().clone().unwrap();
    engine.abort();
    assert!(engine.await.unwrap_err().is_cancelled());
    drop(events);
    // EngineRun retires the suspended future on its dedicated worker.
    tokio::time::timeout(Duration::from_secs(3), async {
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
