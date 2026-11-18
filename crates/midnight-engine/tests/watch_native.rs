use midnight_core::*;
use midnight_engine::{CommandSender, Engine};
use midnight_platform::LocalFileSystem;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};
use tokio::sync::{Notify, mpsc};

/// The real native watcher (ReadDirectoryChangesW on Windows, inotify on Linux)
/// with counted registrations. A test can make selected registrations panic or
/// fail, as a provider fault or an exhausted watch limit would.
struct TrackedWatch {
    filesystem: Arc<LocalFileSystem>,
    calls: AtomicUsize,
    called: Notify,
    registered: AtomicUsize,
    ready: Notify,
    /// The first so many calls panic.
    panic_first: usize,
    /// The next so many calls fail.
    fail_first: usize,
    /// Every call fails while this is set.
    failing: AtomicBool,
}

impl TrackedWatch {
    fn new(filesystem: &Arc<LocalFileSystem>) -> Self {
        Self {
            filesystem: filesystem.clone(),
            calls: AtomicUsize::new(0),
            called: Notify::new(),
            registered: AtomicUsize::new(0),
            ready: Notify::new(),
            panic_first: 0,
            fail_first: 0,
            failing: AtomicBool::new(false),
        }
    }

    /// Waits until `count` registrations were attempted.
    async fn calls(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(8), async {
            while self.calls.load(Ordering::SeqCst) < count {
                self.called.notified().await;
            }
        })
        .await
        .expect("the folder must be registered");
    }

    /// Waits until `count` registrations returned a live native watcher.
    async fn registrations(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(8), async {
            while self.registered.load(Ordering::Acquire) < count {
                self.ready.notified().await;
            }
        })
        .await
        .expect("the native watcher must register");
    }
}

impl WatchService for TrackedWatch {
    fn watch_directory(
        &self,
        path: &Path,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.called.notify_one();
        if call < self.panic_first {
            panic!("watch test registration panic");
        }
        if call < self.panic_first + self.fail_first || self.failing.load(Ordering::SeqCst) {
            return Err(FsError::from_io(
                "watch directory",
                path,
                std::io::Error::other("OS file watch limit reached"),
            ));
        }
        let guard = self.filesystem.watch_directory(path, changed)?;
        self.registered.fetch_add(1, Ordering::Release);
        self.ready.notify_one();
        Ok(guard)
    }
}

/// An engine over a temporary directory whose `left` folder holds one file.
struct RealEngine {
    _temporary: tempfile::TempDir,
    left: PathBuf,
    watch: Arc<TrackedWatch>,
    commands: CommandSender,
    events: mpsc::Receiver<Event>,
    engine: tokio::task::JoinHandle<()>,
}

/// Starts a `RealEngine` watching through a `TrackedWatch` that `configure`
/// sets up.
fn real_engine(configure: impl FnOnce(&mut TrackedWatch)) -> RealEngine {
    let temporary = tempfile::tempdir().unwrap();
    let left = temporary.path().join("left-監視");
    std::fs::create_dir(&left).unwrap();
    std::fs::write(left.join("既存-📄.txt"), b"old").unwrap();
    let filesystem = Arc::new(LocalFileSystem::new(Some(temporary.path().into())));
    let mut watch = TrackedWatch::new(&filesystem);
    configure(&mut watch);
    let watch = Arc::new(watch);
    let (sender, events) = mpsc::channel(32);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        filesystem,
        sender,
    );
    let (commands, receiver) = engine.command_channel(16).unwrap();
    engine.configure_watch().unwrap().install(watch.clone());
    let engine = tokio::spawn(engine.run(receiver));
    RealEngine {
        _temporary: temporary,
        left,
        watch,
        commands,
        events,
        engine,
    }
}

fn navigate(navigation: u64, path: &Path) -> Command {
    Command::Navigate {
        pane: PaneId::Left,
        navigation: NavigationId(navigation),
        path: path.into(),
        sort: SortSpec::default(),
    }
}

/// Waits for a hint for `navigation`; any watch status fails.
async fn hinted_without_status(events: &mut mpsc::Receiver<Event>, navigation: NavigationId) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(8), events.recv())
            .await
            .expect("the watcher must hint the change")
            .expect("engine output");
        match event {
            Event::DirectoryChanged {
                pane,
                navigation: hinted,
            } => {
                assert_eq!((pane, hinted), (PaneId::Left, navigation));
                return;
            }
            Event::WatchStatus { view, status } => {
                panic!("unexpected watch status for {view:?}: {status:?}")
            }
            _ => {}
        }
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
    let watcher = Arc::new(TrackedWatch::new(&filesystem));

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
    watcher.registrations(2).await;
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

/// Directories that each hold one file whose length a test changes. Counts how
/// often each directory is read, and can find the next listing empty, fail the
/// next read of `path`, fail every read of an unreadable directory, or hold one
/// listing after it read its directory.
struct TestDirectory {
    path: PathBuf,
    length: AtomicU64,
    reads: Mutex<HashMap<PathBuf, usize>>,
    empty_next: AtomicBool,
    fail_next: AtomicBool,
    unreadable: Mutex<Option<PathBuf>>,
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
            reads: Mutex::default(),
            empty_next: AtomicBool::new(false),
            fail_next: AtomicBool::new(false),
            unreadable: Mutex::default(),
            hold_next: AtomicBool::new(false),
            held: Notify::new(),
            resumed: Mutex::new(false),
            resume: Condvar::new(),
        }
    }

    /// Reads of the directory the pane starts in.
    fn reads(&self) -> usize {
        self.reads_of(&self.path)
    }

    fn reads_of(&self, path: &Path) -> usize {
        self.reads.lock().unwrap().get(path).copied().unwrap_or(0)
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
        *self.reads.lock().unwrap().entry(path.into()).or_default() += 1;
        if (path == self.path && self.fail_next.swap(false, Ordering::SeqCst))
            || self.unreadable.lock().unwrap().as_deref() == Some(path)
        {
            return Err(FsError::from_io(
                "read directory",
                path,
                std::io::ErrorKind::PermissionDenied.into(),
            ));
        }
        if self.empty_next.swap(false, Ordering::SeqCst) {
            return Ok(());
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

type Callback = Arc<dyn Fn() + Send + Sync>;

/// Notifies only when the test fires a recorded subscription.
#[derive(Default)]
struct ManualWatch {
    callbacks: Mutex<Vec<(PathBuf, Callback)>>,
    registered: Notify,
    /// Fires the newest subscription when the next one is registered: a
    /// notification that arrives while the watcher subscribes again.
    fire_on_watch: AtomicBool,
}

struct ManualSubscription;
impl WatchSubscription for ManualSubscription {}

impl WatchService for ManualWatch {
    fn watch_directory(
        &self,
        path: &Path,
        changed: Callback,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        if self.fire_on_watch.swap(false, Ordering::SeqCst) {
            let newest = self
                .callbacks
                .lock()
                .unwrap()
                .last()
                .map(|(_, callback)| callback.clone());
            if let Some(callback) = newest {
                callback();
            }
        }
        self.callbacks.lock().unwrap().push((path.into(), changed));
        self.registered.notify_one();
        Ok(Box::new(ManualSubscription))
    }
}

impl ManualWatch {
    /// Waits until subscription `index` exists and returns its directory and
    /// callback.
    async fn watched(&self, index: usize) -> (PathBuf, Callback) {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let subscription = self.callbacks.lock().unwrap().get(index).cloned();
                if let Some(subscription) = subscription {
                    return subscription;
                }
                self.registered.notified().await;
            }
        })
        .await
        .expect("the directory must be watched")
    }

    /// Waits until subscription `index` exists and returns its callback.
    async fn subscription(&self, index: usize) -> Callback {
        self.watched(index).await.1
    }

    async fn fire(&self, index: usize) {
        let callback = self.subscription(index).await;
        callback();
    }
}

/// Starts an engine over `directory` whose left pane shows it as navigation 1.
/// That first listing finds the directory empty, so it is not subscribed while
/// it is listed: subscription 0 is registered when the rows are accepted, and
/// firing it is hinted. (A notification while a folder's first listing is
/// read is checked when the rows are accepted instead.)
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
    directory.empty_next.store(true, Ordering::SeqCst);
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

/// A resort of the rows `source` shows, or without rows to reuse.
fn resort(
    navigation: u64,
    directory: &TestDirectory,
    source: Option<(NavigationId, Arc<[Entry]>)>,
) -> Command {
    Command::Resort {
        pane: PaneId::Left,
        navigation: NavigationId(navigation),
        path: directory.path.clone(),
        sort: SortSpec::default(),
        source: source.map(|(navigation, entries)| VisibleDirectory {
            pane: PaneId::Left,
            navigation,
            path: directory.path.clone(),
            entries,
        }),
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

/// Waits for the left pane's listing of `navigation` and accepts it as the UI
/// does. Returns the accepted rows.
async fn commit(
    commands: &CommandSender,
    events: &mut mpsc::Receiver<Event>,
    navigation: NavigationId,
    path: &Path,
) -> Arc<[Entry]> {
    let entries = listed(events, navigation).await;
    accept(commands, navigation, path, entries.clone()).await;
    entries
}

/// Waits for the rows of the left pane's listing of `navigation`.
async fn listed(events: &mut mpsc::Receiver<Event>, navigation: NavigationId) -> Arc<[Entry]> {
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
            return entries;
        }
    }
}

/// The UI accepts the left pane's rows of `navigation`.
async fn accept(
    commands: &CommandSender,
    navigation: NavigationId,
    path: &Path,
    entries: Arc<[Entry]>,
) {
    commands
        .send(Command::PaneCommitted(VisibleDirectory {
            pane: PaneId::Left,
            navigation,
            path: path.into(),
            entries,
        }))
        .await
        .unwrap();
}

/// Waits for the first engine event that `wanted` accepts.
async fn next(events: &mut mpsc::Receiver<Event>, expected: &str, wanted: fn(&Event) -> bool) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !wanted(&events.recv().await.expect("engine output")) {}
    })
    .await
    .unwrap_or_else(|_| panic!("{expected}"));
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

/// The UI lists the directory again for a hint, with `listing` as navigation 2,
/// and accepts rows that already include the notified change.
async fn assert_an_event_driven_listing_reads_once(listing: fn(u64, &TestDirectory) -> Command) {
    let directory = Arc::new(TestDirectory::new(Path::new("監視-📁")));
    let watch = Arc::new(ManualWatch::default());
    let (commands, mut events, engine) = start(&directory, &watch).await;
    watch.fire(0).await;
    hinted(&mut events, NavigationId(1)).await;
    commands.send(listing(2, &directory)).await.unwrap();
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
async fn an_event_driven_refresh_lists_its_directory_once() {
    assert_an_event_driven_listing_reads_once(refresh).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resort_without_the_ui_rows_lists_its_directory_once() {
    // The backend lists the directory for it, so it is fenced like a refresh.
    assert_an_event_driven_listing_reads_once(|navigation, directory| {
        resort(navigation, directory, None)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resort_of_rows_a_check_found_current_reads_nothing() {
    let directory = Arc::new(TestDirectory::new(Path::new("監視-📁")));
    let watch = Arc::new(ManualWatch::default());
    let (commands, mut events, engine) = start(&directory, &watch).await;
    let watched = watch.subscription(0).await;
    // The user refreshes after a write whose notification is counted only after
    // the refresh started, so the accepted rows are checked once.
    directory.length.store(8, Ordering::SeqCst);
    commands.send(refresh(2, &directory)).await.unwrap();
    next(&mut events, "the refresh must start", |event| {
        matches!(
            event,
            Event::ListingStarted {
                pane: PaneId::Left,
                navigation: NavigationId(2),
                ..
            }
        )
    })
    .await;
    watched();
    // Another notification arrives while the accepted rows are subscribed
    // again; it is hinted after that check.
    watch.fire_on_watch.store(true, Ordering::SeqCst);
    let rows = commit(&commands, &mut events, NavigationId(2), &directory.path).await;
    hinted(&mut events, NavigationId(2)).await;
    assert_eq!(directory.reads(), 3, "two listings and one check");
    // The check found the rows current, so re-sorting them reads nothing.
    let source = Some((NavigationId(2), rows));
    commands.send(resort(3, &directory, source)).await.unwrap();
    commit(&commands, &mut events, NavigationId(3), &directory.path).await;
    watch.fire(2).await;
    hinted(&mut events, NavigationId(3)).await;
    assert_eq!(directory.reads(), 3);
    shut_down(commands, events, engine).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_refresh_keeps_the_shown_directory_watched() {
    let directory = Arc::new(TestDirectory::new(Path::new("監視-📁")));
    let watch = Arc::new(ManualWatch::default());
    let (commands, mut events, engine) = start(&directory, &watch).await;
    let watched = watch.subscription(0).await;
    directory.fail_next.store(true, Ordering::SeqCst);
    commands.send(refresh(2, &directory)).await.unwrap();
    next(&mut events, "the refresh must fail", |event| {
        matches!(
            event,
            Event::ListingFailed {
                pane: PaneId::Left,
                navigation: NavigationId(2),
                ..
            }
        )
    })
    .await;
    // The pane still shows navigation 1, and its directory is still watched.
    watched();
    hinted(&mut events, NavigationId(1)).await;
    assert_eq!(
        watch.callbacks.lock().unwrap().len(),
        1,
        "the directory stayed subscribed"
    );
    assert_eq!(
        directory.reads(),
        2,
        "one listing and the failed refresh, which is not checked again at once"
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_navigation_to_another_directory_keeps_the_shown_directory_watched() {
    let directory = Arc::new(TestDirectory::new(Path::new("監視-📁")));
    let unreadable = PathBuf::from("無権限-🔒");
    *directory.unreadable.lock().unwrap() = Some(unreadable.clone());
    let watch = Arc::new(ManualWatch::default());
    let (commands, mut events, engine) = start(&directory, &watch).await;
    let watched = watch.subscription(0).await;
    commands.send(navigate(2, &unreadable)).await.unwrap();
    next(&mut events, "the navigation must fail", |event| {
        matches!(
            event,
            Event::ListingFailed {
                pane: PaneId::Left,
                navigation: NavigationId(2),
                ..
            }
        )
    })
    .await;
    // The pane still shows navigation 1, and its directory is still watched.
    watched();
    hinted(&mut events, NavigationId(1)).await;
    assert_eq!(watch.callbacks.lock().unwrap().len(), 1);
    assert_eq!(
        directory.reads(),
        1,
        "one listing, which is not checked again"
    );
    assert_eq!(directory.reads_of(&unreadable), 1, "one failed listing");
    shut_down(commands, events, engine).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_navigation_keeps_real_native_hints_for_the_shown_directory() {
    let RealEngine {
        _temporary,
        left,
        watch,
        commands,
        mut events,
        engine,
    } = real_engine(|_| {});
    commands.send(navigate(1, &left)).await.unwrap();
    commit(&commands, &mut events, NavigationId(1), &left).await;
    watch.registrations(1).await;
    // The test engine has no navigation fallback, so the missing folder fails.
    commands
        .send(navigate(2, &left.join("missing-文档")))
        .await
        .unwrap();
    next(&mut events, "the navigation must fail", |event| {
        matches!(
            event,
            Event::ListingFailed {
                pane: PaneId::Left,
                navigation: NavigationId(2),
                ..
            }
        )
    })
    .await;
    // Only native evidence can hint within the wait: the check runs every 15 s.
    std::fs::write(left.join("新-👩🏽‍💻.txt"), b"new data").unwrap();
    hinted_without_status(&mut events, NavigationId(1)).await;
    shut_down(commands, events, engine).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_while_a_new_directory_is_listed_is_hinted_once_it_is_shown() {
    let directory = Arc::new(TestDirectory::new(Path::new("監視-📁")));
    let resume = ResumeOnDrop(directory.clone());
    let watch = Arc::new(ManualWatch::default());
    let (commands, mut events, engine) = start(&directory, &watch).await;
    watch.subscription(0).await;
    let second = PathBuf::from("第二-📂");
    // The listing reads the second folder's first entry and is held there.
    directory.hold_next.store(true, Ordering::SeqCst);
    commands.send(navigate(2, &second)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), directory.held.notified())
        .await
        .expect("the navigation must list the folder");
    // The folder is subscribed while it is listed.
    let (path, notify) = watch.watched(1).await;
    assert_eq!(path, second);
    // A write lands after the listing read the folder and is notified while
    // the UI is still loading.
    directory.length.store(8, Ordering::SeqCst);
    notify();
    drop(resume);
    commit(&commands, &mut events, NavigationId(2), &second).await;
    hinted(&mut events, NavigationId(2)).await;
    assert_eq!(directory.reads_of(&second), 2, "one listing and one check");
    assert_eq!(
        directory.reads(),
        1,
        "the folder shown before is not checked"
    );
    shut_down(commands, events, engine).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_preparation_is_registered_again_when_the_folder_is_shown() {
    let RealEngine {
        _temporary,
        left,
        watch,
        commands,
        mut events,
        engine,
    } = real_engine(|watch| watch.fail_first = 1);
    commands.send(navigate(1, &left)).await.unwrap();
    let rows = listed(&mut events, NavigationId(1)).await;
    // The listing read an entry, so the folder was subscribed while it was
    // listed, before the UI accepts the rows; that registration failed.
    watch.calls(1).await;
    accept(&commands, NavigationId(1), &left, rows).await;
    // It is registered once more when the rows are shown, without a status.
    watch.registrations(1).await;
    std::fs::write(left.join("新-👩🏽‍💻.txt"), b"new data").unwrap();
    hinted_without_status(&mut events, NavigationId(1)).await;
    assert_eq!(watch.calls.load(Ordering::SeqCst), 2);
    shut_down(commands, events, engine).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_registration_reports_polling_only_until_a_refresh_registers() {
    let RealEngine {
        _temporary,
        left,
        watch,
        commands,
        mut events,
        engine,
    } = real_engine(|watch| watch.failing = AtomicBool::new(true));
    commands.send(navigate(1, &left)).await.unwrap();
    commit(&commands, &mut events, NavigationId(1), &left).await;
    // Every registration of the folder fails (its preparation, then the one
    // when it is shown); the status is reported once.
    let status = loop {
        let event = tokio::time::timeout(Duration::from_secs(8), events.recv())
            .await
            .expect("the polling-only status must be reported")
            .expect("engine output");
        if let Event::WatchStatus { view, status } = event {
            break (view, status);
        }
    };
    assert_eq!(
        status.0,
        WatchedView::Flat {
            pane: PaneId::Left,
            navigation: NavigationId(1),
        }
    );
    assert!(
        matches!(&status.1, WatchStatus::PollingOnly(error) if error.path == left),
        "{status:?}"
    );
    // A refresh of the folder registers it again, as a new view that starts
    // natively watched: no status is sent for it.
    watch.failing.store(false, Ordering::SeqCst);
    commands
        .send(Command::Refresh {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: left.clone(),
            sort: SortSpec::default(),
            scan: None,
        })
        .await
        .unwrap();
    commit(&commands, &mut events, NavigationId(2), &left).await;
    watch.registrations(1).await;
    std::fs::write(left.join("新-👩🏽‍💻.txt"), b"new data").unwrap();
    hinted_without_status(&mut events, NavigationId(2)).await;
    shut_down(commands, events, engine).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panicking_watch_registration_restarts_the_pane_watcher() {
    let RealEngine {
        _temporary,
        left,
        watch,
        commands,
        mut events,
        engine,
    } = real_engine(|watch| watch.panic_first = 1);
    commands.send(navigate(1, &left)).await.unwrap();
    commit(&commands, &mut events, NavigationId(1), &left).await;
    // The worker restarts at once and registers the folder again. (If the
    // write is notified before the rows are accepted, the folder is also
    // subscribed again at acceptance.)
    watch.registrations(1).await;
    std::fs::write(left.join("新-👩🏽‍💻.txt"), b"new data").unwrap();
    hinted_without_status(&mut events, NavigationId(1)).await;
    assert!(watch.calls.load(Ordering::SeqCst) >= 2);
    shut_down(commands, events, engine).await;
}
