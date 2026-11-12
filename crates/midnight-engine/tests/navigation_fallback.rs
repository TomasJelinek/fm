use midnight_core::{
    Cancellation, Command, DirectoryService, Entry, EntryMetadata, Event, FsError,
    NavigationFallbackService, NavigationId, PaneId, SortSpec, SpaceInfo, StorageService, Volume,
    VolumeService,
};
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use tokio::sync::{Notify, mpsc};

fn navigate(generation: u64, path: PathBuf) -> Command {
    Command::Navigate {
        pane: PaneId::Left,
        navigation: NavigationId(generation),
        path,
        sort: SortSpec::default(),
    }
}

async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_unicode_descendants_recover_each_parent_and_list_the_first_existing_ancestor() {
    let fixture = tempfile::tempdir().unwrap();
    let accessible = fixture.path().join("Příliš_中文_🧪_e\u{301}");
    std::fs::create_dir(&accessible).unwrap();
    std::fs::write(accessible.join("kept.txt"), b"untouched").unwrap();
    let parent = accessible.join("missing");
    let missing = parent.join("child");
    let filesystem = Arc::new(LocalFileSystem::new(None));
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .with_navigation_fallback(filesystem)
        .run(receiver),
    );
    commands.send(navigate(1, missing)).await.unwrap();
    let mut fallbacks = Vec::new();
    loop {
        match next(&mut events).await {
            Event::ListingFallback {
                navigation, path, ..
            } => {
                fallbacks.push(path.clone());
                assert!(
                    fallbacks.len() <= 2,
                    "recovery must stop at the accessible ancestor"
                );
                commands
                    .send(navigate(navigation.0 + 1, path))
                    .await
                    .unwrap();
            }
            Event::ListingFinished {
                navigation,
                entries,
                ..
            } => {
                assert_eq!(navigation, NavigationId(3));
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].path, accessible.join("kept.txt"));
                break;
            }
            Event::ListingFailed { error, .. } => panic!("{error}"),
            _ => {}
        }
    }
    assert_eq!(fallbacks, [parent, accessible]);
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

struct FailingFileSystem {
    root: PathBuf,
    blocked: PathBuf,
    entered: Notify,
    release: (Mutex<bool>, Condvar),
}

impl DirectoryService for FailingFileSystem {
    fn read_directory(
        &self,
        path: &Path,
        _cancel: &Cancellation,
        _emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if path == self.blocked {
            self.entered.notify_one();
            let (lock, condition) = &self.release;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = condition.wait(released).unwrap();
            }
        }
        Err(FsError::from_io(
            "list",
            path,
            io::ErrorKind::PermissionDenied.into(),
        ))
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        Err(FsError::from_io(
            "metadata",
            path,
            io::ErrorKind::PermissionDenied.into(),
        ))
    }
}
impl NavigationFallbackService for FailingFileSystem {
    fn navigation_fallback_root(&self) -> Result<PathBuf, FsError> {
        Ok(self.root.clone())
    }
}
impl VolumeService for FailingFileSystem {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for FailingFileSystem {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        Err(FsError::from_io(
            "space",
            path,
            io::ErrorKind::PermissionDenied.into(),
        ))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_uncancellable_failure_emits_no_fallback_and_failed_root_stops() {
    let root = std::env::current_dir()
        .unwrap()
        .ancestors()
        .last()
        .unwrap()
        .to_owned();
    let blocked = root.join("blocked");
    let filesystem = Arc::new(FailingFileSystem {
        root: root.clone(),
        blocked: blocked.clone(),
        entered: Notify::new(),
        release: (Mutex::new(false), Condvar::new()),
    });
    struct Release(Arc<FailingFileSystem>);
    impl Drop for Release {
        fn drop(&mut self) {
            *self.0.release.0.lock().unwrap() = true;
            self.0.release.1.notify_all();
        }
    }
    let release = Release(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .with_navigation_fallback(filesystem.clone())
        .run(receiver),
    );
    commands.send(navigate(1, blocked)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), filesystem.entered.notified())
        .await
        .unwrap();
    commands.send(navigate(2, root.clone())).await.unwrap();
    loop {
        match next(&mut events).await {
            Event::ListingFallback { .. } => {
                panic!("neither cancelled work nor the final root retries")
            }
            Event::ListingFailed {
                navigation, error, ..
            } => {
                assert_eq!(navigation, NavigationId(2));
                assert_eq!(error.path, root);
                break;
            }
            _ => {}
        }
    }
    drop(release);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                if let Some(Event::ListingFallback { .. } | Event::ListingFailed { .. }) =
                    events.recv().await
                {
                    panic!("late cancelled error must not restart navigation");
                }
            }
        })
        .await
        .is_err()
    );
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}
