use midnight_core::*;
use midnight_engine::{CommandSender, Engine};
use midnight_platform::LocalFileSystem;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};
use tokio::sync::{Notify, mpsc};

struct TrackedWatch {
    filesystem: Arc<LocalFileSystem>,
    registered: AtomicUsize,
    ready: Notify,
}

impl WatchService for TrackedWatch {
    fn watch_directory(
        &self,
        path: &Path,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        let guard = self.filesystem.watch_directory(path, changed)?;
        self.registered.fetch_add(1, Ordering::Release);
        self.ready.notify_one();
        Ok(guard)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_panes_receive_real_native_change_hints_and_can_reload() {
    let temporary = tempfile::tempdir().unwrap();
    let left = temporary.path().join("left");
    let right = temporary.path().join("right");
    std::fs::create_dir(&left).unwrap();
    std::fs::create_dir(&right).unwrap();
    let filesystem = Arc::new(LocalFileSystem::new(Some(temporary.path().into())));
    let watcher = Arc::new(TrackedWatch {
        filesystem: filesystem.clone(),
        registered: AtomicUsize::new(0),
        ready: Notify::new(),
    });

    let (sender, mut events) = mpsc::channel(32);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        filesystem,
        sender,
    );
    let (commands, receiver) = engine.command_channel(16).unwrap();
    engine.configure_watch().unwrap().install(watcher.clone());
    let engine = tokio::spawn(engine.run(receiver));
    for (pane, path) in [(PaneId::Left, &left), (PaneId::Right, &right)] {
        commands
            .send(Command::Navigate {
                pane,
                navigation: NavigationId(1),
                path: path.clone(),
                sort: SortSpec::default(),
            })
            .await
            .unwrap();
    }
    let mut accepted = 0;
    while accepted < 2 {
        let event = tokio::time::timeout(Duration::from_secs(8), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let Event::ListingFinished {
            pane,
            navigation,
            entries,
            ..
        } = event
        {
            let path = if pane == PaneId::Left {
                left.clone()
            } else {
                right.clone()
            };
            commands
                .send(Command::PaneCommitted(VisibleDirectory {
                    pane,
                    navigation,
                    path,
                    entries,
                }))
                .await
                .unwrap();
            accepted += 1;
        }
    }
    tokio::time::timeout(Duration::from_secs(8), async {
        while watcher.registered.load(Ordering::Acquire) < 2 {
            watcher.ready.notified().await;
        }
    })
    .await
    .expect("Both native watchers must register");
    std::fs::write(left.join("Žluťoučký-文档-👩🏽‍💻.txt"), b"new data").unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Event::DirectoryChanged { pane, navigation } = events.recv().await.unwrap() {
                assert_eq!((pane, navigation), (PaneId::Left, NavigationId(1)));
                break;
            }
        }
    })
    .await
    .expect("Native event must survive debounce and reach the accepted pane");
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: left.clone(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Event::ListingFinished {
                pane: PaneId::Left,
                navigation: NavigationId(2),
                entries,
                ..
            } = events.recv().await.unwrap()
            {
                assert!(
                    entries
                        .iter()
                        .any(|entry| entry.path == left.join("Žluťoučký-文档-👩🏽‍💻.txt")
                            && entry.metadata.size == Some(8))
                );
                break;
            }
        }
    })
    .await
    .unwrap();
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    engine.await.unwrap();
}

/// One directory holding one file whose length a test changes. Counts how often
/// the directory is read and can hold one listing after it read the directory.
struct TestDirectory {
    path: PathBuf,
    length: AtomicU64,
    reads: AtomicUsize,
    hold_next: AtomicBool,
    held: Notify,
    resumed: Mutex<bool>,
    resume: Condvar,
}

impl TestDirectory {
    fn new(path: &Path) -> Self {
        Self {
            path: path.into(),
            length: AtomicU64::new(7),
            reads: AtomicUsize::new(0),
            hold_next: AtomicBool::new(false),
            held: Notify::new(),
            resumed: Mutex::new(false),
            resume: Condvar::new(),
        }
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

/// Lets a held listing finish, also when a test fails early.
struct ResumeOnDrop(Arc<TestDirectory>);

impl Drop for ResumeOnDrop {
    fn drop(&mut self) {
        *self
            .0
            .resumed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.0.resume.notify_all();
    }
}

impl DirectoryService for TestDirectory {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if path == self.path {
            self.reads.fetch_add(1, Ordering::SeqCst);
        }
        let mut entry = Entry::new(EntryId(1), path.join("Žluťoučký-文档.txt"), EntryKind::File);
        entry.metadata.size = Some(self.length.load(Ordering::SeqCst));
        entry.metadata.modified = Some(SystemTime::UNIX_EPOCH);
        emit(entry);
        if self.hold_next.swap(false, Ordering::SeqCst) {
            self.held.notify_one();
            let mut resumed = self.resumed.lock().unwrap();
            while !*resumed && !cancel.is_cancelled() {
                resumed = self
                    .resume
                    .wait_timeout(resumed, Duration::from_millis(10))
                    .unwrap()
                    .0;
            }
        }
        Ok(())
    }

    fn listing_metadata_complete(&self, _entry: &Entry) -> bool {
        true
    }

    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        Err(FsError::from_io(
            "entry metadata",
            path,
            std::io::ErrorKind::NotFound.into(),
        ))
    }
}

impl VolumeService for TestDirectory {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}

impl StorageService for TestDirectory {
    fn space_info(&self, _path: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            available: 1,
            total: 2,
        })
    }
}

/// Notifies only when the test fires a recorded subscription.
#[derive(Default)]
struct ManualWatch {
    callbacks: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
    registered: Notify,
}

struct ManualSubscription;
impl WatchSubscription for ManualSubscription {}

impl WatchService for ManualWatch {
    fn watch_directory(
        &self,
        _path: &Path,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        self.callbacks.lock().unwrap().push(changed);
        self.registered.notify_one();
        Ok(Box::new(ManualSubscription))
    }
}

impl ManualWatch {
    /// Waits until subscription `index` exists and returns its callback.
    async fn subscription(&self, index: usize) -> Arc<dyn Fn() + Send + Sync> {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let callback = self.callbacks.lock().unwrap().get(index).cloned();
                if let Some(callback) = callback {
                    return callback;
                }
                self.registered.notified().await;
            }
        })
        .await
        .expect("the accepted directory must be watched")
    }

    async fn fire(&self, index: usize) {
        let callback = self.subscription(index).await;
        callback();
    }
}

/// Starts an engine over `directory` whose left pane shows it as navigation 1.
async fn start(
    directory: &Arc<TestDirectory>,
    watch: &Arc<ManualWatch>,
) -> (
    CommandSender,
    mpsc::Receiver<Event>,
    tokio::task::JoinHandle<()>,
) {
    let (sender, mut events) = mpsc::channel(32);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        directory.clone(),
        sender,
    );
    let (commands, receiver) = engine.command_channel(16).unwrap();
    engine.configure_watch().unwrap().install(watch.clone());
    let engine = tokio::spawn(engine.run(receiver));
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: directory.path.clone(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    commit(&commands, &mut events, NavigationId(1), &directory.path).await;
    (commands, events, engine)
}

fn refresh(navigation: u64, directory: &TestDirectory) -> Command {
    Command::Refresh {
        pane: PaneId::Left,
        navigation: NavigationId(navigation),
        path: directory.path.clone(),
        sort: SortSpec::default(),
        scan: None,
    }
}

async fn shut_down(
    commands: CommandSender,
    events: mpsc::Receiver<Event>,
    engine: tokio::task::JoinHandle<()>,
) {
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    engine.await.unwrap();
}

/// Waits for the left pane's listing of `navigation` and accepts it as the UI does.
async fn commit(
    commands: &CommandSender,
    events: &mut mpsc::Receiver<Event>,
    navigation: NavigationId,
    path: &Path,
) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(8), events.recv())
            .await
            .expect("the listing must finish")
            .expect("engine output");
        if let Event::ListingFinished {
            pane: PaneId::Left,
            navigation: finished,
            entries,
            ..
        } = event
            && finished == navigation
        {
            commands
                .send(Command::PaneCommitted(VisibleDirectory {
                    pane: PaneId::Left,
                    navigation,
                    path: path.into(),
                    entries,
                }))
                .await
                .unwrap();
            return;
        }
    }
}

async fn hinted(events: &mut mpsc::Receiver<Event>, navigation: NavigationId) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(8), events.recv())
            .await
            .expect("the watcher must hint the change")
            .expect("engine output");
        if let Event::DirectoryChanged {
            pane,
            navigation: hinted,
        } = event
        {
            assert_eq!((pane, hinted), (PaneId::Left, navigation));
            return;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_event_driven_refresh_lists_its_directory_once() {
    let directory = Arc::new(TestDirectory::new(Path::new("監視-📁")));
    let watch = Arc::new(ManualWatch::default());
    let (commands, mut events, engine) = start(&directory, &watch).await;
    watch.fire(0).await;
    hinted(&mut events, NavigationId(1)).await;
    // The UI refreshes for the hint and accepts the new listing, which already
    // includes the notified change.
    commands.send(refresh(2, &directory)).await.unwrap();
    commit(&commands, &mut events, NavigationId(2), &directory.path).await;
    // The acceptance decides whether to check the new rows before it subscribes
    // again, and such a check runs before a later notification can be hinted.
    watch.fire(1).await;
    hinted(&mut events, NavigationId(2)).await;
    assert_eq!(
        directory.reads(),
        2,
        "each listing reads the directory once, with no further check"
    );
    shut_down(commands, events, engine).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_during_a_refresh_listing_is_hinted_for_the_new_view() {
    let directory = Arc::new(TestDirectory::new(Path::new("監視-📁")));
    let resume = ResumeOnDrop(directory.clone());
    let watch = Arc::new(ManualWatch::default());
    let (commands, mut events, engine) = start(&directory, &watch).await;
    let watched = watch.subscription(0).await;
    // The user refreshes; the listing reads the directory and is held there.
    directory.hold_next.store(true, Ordering::SeqCst);
    commands.send(refresh(2, &directory)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), directory.held.notified())
        .await
        .expect("the refresh must list the directory");
    // A write lands after the listing read the directory and is notified while
    // the UI is still loading.
    directory.length.store(8, Ordering::SeqCst);
    watched();
    drop(resume);
    commit(&commands, &mut events, NavigationId(2), &directory.path).await;
    // The directory stayed subscribed, so the accepted rows are checked at once.
    hinted(&mut events, NavigationId(2)).await;
    assert_eq!(directory.reads(), 3, "two listings and one check");
    shut_down(commands, events, engine).await;
}
