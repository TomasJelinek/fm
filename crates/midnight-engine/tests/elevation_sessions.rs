//! One consent lists the authorized folder and, through the pane's session, the folders
//! inside it: the engine routes a denied listing of such a folder through the session
//! without another prompt, and ends the session when the pane leaves the folder.
use midnight_core::{
    Cancellation, Command, DirectoryService, ElevatedDirectoryService, ElevatedListingSession,
    ElevationAvailability, Entry, EntryId, EntryKind, EntryMetadata, Event, FsError, NavigationId,
    PaneId, SessionListingError, SortSpec, SpaceInfo, StorageService, VisibleDirectory, Volume,
    VolumeService, WatchService, WatchSubscription,
};
use midnight_engine::{CommandSender, Engine};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

/// Denies every folder to this process's own rights.
#[derive(Default)]
struct Denied(Mutex<Vec<PathBuf>>);
impl DirectoryService for Denied {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.0.lock().unwrap().push(path.to_owned());
        Err(FsError::from_io(
            "list",
            path,
            io::ErrorKind::PermissionDenied.into(),
        ))
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        panic!("helper metadata must not be reread with ordinary privileges: {path:?}")
    }
}
impl VolumeService for Denied {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for Denied {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        Err(FsError::from_io(
            "space",
            path,
            io::ErrorKind::PermissionDenied.into(),
        ))
    }
}

/// What the helper fake was asked.
#[derive(Default)]
struct Calls {
    /// Consents: each authorized retry.
    opened: Vec<PathBuf>,
    /// Listings through a session.
    listed: Vec<PathBuf>,
    /// Sessions dropped.
    closed: usize,
}

struct Sessions {
    calls: Arc<Mutex<Calls>>,
    /// Folders the helper finds outside the authorized one.
    outside: Vec<PathBuf>,
    /// Every session has ended (its helper idled out).
    ended: Arc<AtomicBool>,
}
impl ElevatedDirectoryService for Sessions {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        panic!("an authorized retry opens a session")
    }
    fn open_session(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<Option<Box<dyn ElevatedListingSession>>, FsError> {
        self.calls.lock().unwrap().opened.push(path.to_owned());
        self.ended.store(false, Ordering::SeqCst);
        emit(file(path, "visible.txt"));
        Ok(Some(Box::new(FakeSession {
            root: path.to_owned(),
            calls: self.calls.clone(),
            outside: self.outside.clone(),
            ended: self.ended.clone(),
        })))
    }
}

fn file(folder: &Path, name: &str) -> Entry {
    let mut entry = Entry::new(EntryId(1), folder.join(name), EntryKind::File);
    entry.metadata.size = Some(7);
    entry
}

struct FakeSession {
    root: PathBuf,
    calls: Arc<Mutex<Calls>>,
    outside: Vec<PathBuf>,
    ended: Arc<AtomicBool>,
}
impl ElevatedListingSession for FakeSession {
    fn root(&self) -> &Path {
        &self.root
    }
    fn is_live(&self) -> bool {
        !self.ended.load(Ordering::SeqCst)
    }
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), SessionListingError> {
        if self.ended.load(Ordering::SeqCst) {
            return Err(SessionListingError::Ended);
        }
        self.calls.lock().unwrap().listed.push(path.to_owned());
        if self.outside.iter().any(|outside| outside == path) {
            return Err(SessionListingError::Outside(
                FsError::outside_authorization(path, &self.root, "The fake helper refused it."),
            ));
        }
        emit(file(path, "inside.txt"));
        Ok(())
    }
}
impl Drop for FakeSession {
    fn drop(&mut self) {
        self.calls.lock().unwrap().closed += 1;
    }
}

/// Refuses every native watch, as `ReadDirectoryChangesW` refuses a denied folder.
#[derive(Default)]
struct DeniedWatch(Mutex<Vec<PathBuf>>);
impl WatchService for DeniedWatch {
    fn watch_directory(
        &self,
        path: &Path,
        _: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        self.0.lock().unwrap().push(path.to_owned());
        Err(FsError::from_io(
            "watch directory",
            path,
            io::ErrorKind::PermissionDenied.into(),
        ))
    }
}

struct Harness {
    commands: CommandSender,
    events: mpsc::Receiver<Event>,
    task: tokio::task::JoinHandle<()>,
    calls: Arc<Mutex<Calls>>,
    ended: Arc<AtomicBool>,
    filesystem: Arc<Denied>,
    watch: Arc<DeniedWatch>,
    root: PathBuf,
}

/// An engine whose left pane shows `root` listed through an authorized retry (navigation
/// 2), which opened the pane's session.
async fn authorized(outside: &[&str]) -> Harness {
    let root = std::env::current_dir().unwrap().join("denied-session-授权");
    let (sender, events) = mpsc::channel(64);
    let calls = Arc::new(Mutex::new(Calls::default()));
    let ended = Arc::new(AtomicBool::new(false));
    let filesystem = Arc::new(Denied::default());
    let watch = Arc::new(DeniedWatch::default());
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        filesystem.clone(),
        sender,
    );
    let (commands, receiver) = engine.command_channel(16).unwrap();
    engine
        .configure_elevated_directories()
        .unwrap()
        .install(Arc::new(Sessions {
            calls: calls.clone(),
            outside: outside.iter().map(|name| root.join(name)).collect(),
            ended: ended.clone(),
        }));
    engine.configure_watch().unwrap().install(watch.clone());
    let task = tokio::spawn(engine.run(receiver));
    let mut harness = Harness {
        commands,
        events,
        task,
        calls,
        ended,
        filesystem,
        watch,
        root,
    };
    harness.navigate(1, &harness.root.clone()).await;
    harness.offered(1).await;
    harness
        .commands
        .send(Command::RetryDirectoryWithElevation {
            pane: PaneId::Left,
            denied_navigation: NavigationId(1),
            navigation: NavigationId(2),
        })
        .await
        .unwrap();
    let rows = harness.authorized_rows(2).await;
    assert_eq!(
        rows[0].native_path(),
        Some(harness.root.join("visible.txt").as_path())
    );
    harness
}

/// How a listing ended.
#[derive(Debug)]
enum Outcome {
    /// Its rows, and whether `ListingAuthorized` preceded them.
    Listed(Arc<[Entry]>, bool),
    /// Its failure, and the availability offered after it, if any.
    Failed(FsError, Option<ElevationAvailability>),
}

impl Harness {
    async fn next(&mut self) -> Event {
        tokio::time::timeout(Duration::from_secs(5), self.events.recv())
            .await
            .unwrap()
            .unwrap()
    }

    async fn navigate(&self, navigation: u64, path: &Path) {
        self.commands
            .send(Command::Navigate {
                pane: PaneId::Left,
                navigation: NavigationId(navigation),
                path: path.to_owned(),
                sort: SortSpec::default(),
            })
            .await
            .unwrap();
    }

    /// The outcome of the listing dispatched as `navigation`.
    async fn outcome(&mut self, navigation: u64) -> Outcome {
        let navigation = NavigationId(navigation);
        let mut authorized = false;
        loop {
            match self.next().await {
                Event::ListingAuthorized {
                    pane: PaneId::Left,
                    navigation: marked,
                } if marked == navigation => authorized = true,
                Event::ListingFinished {
                    navigation: finished,
                    entries,
                    ..
                } if finished == navigation => return Outcome::Listed(entries, authorized),
                Event::ListingFailed {
                    navigation: failed,
                    error,
                    ..
                } if failed == navigation => {
                    assert!(!authorized, "a failed listing was marked authorized");
                    // The offer, if any, follows the failure at once.
                    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
                    let mut offered = None;
                    while let Ok(Some(event)) =
                        tokio::time::timeout_at(deadline, self.events.recv()).await
                    {
                        if let Event::DirectoryElevationAvailable {
                            navigation: offer,
                            availability,
                            ..
                        } = event
                            && offer == navigation
                        {
                            offered = Some(availability);
                            break;
                        }
                    }
                    return Outcome::Failed(error, offered);
                }
                Event::ListingFallback { path, .. } => panic!("the pane moved to {path:?}"),
                _ => {}
            }
        }
    }

    async fn offered(&mut self, navigation: u64) {
        match self.outcome(navigation).await {
            Outcome::Failed(error, Some(ElevationAvailability::Available)) => {
                assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
            }
            other => panic!("navigation {navigation} offered no retry: {other:?}"),
        }
    }

    async fn authorized_rows(&mut self, navigation: u64) -> Arc<[Entry]> {
        match self.outcome(navigation).await {
            Outcome::Listed(rows, marked) => {
                // The retry's own rows need no mark: the UI sent the retry.
                assert!(
                    marked || navigation == 2,
                    "navigation {navigation} was not marked"
                );
                rows
            }
            other => panic!("navigation {navigation} was not listed: {other:?}"),
        }
    }

    fn calls(&self) -> std::sync::MutexGuard<'_, Calls> {
        self.calls.lock().unwrap()
    }

    async fn shutdown(self) -> (Arc<Mutex<Calls>>, Arc<Denied>, Arc<DeniedWatch>) {
        self.commands.send(Command::Shutdown).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .unwrap()
            .unwrap();
        (self.calls, self.filesystem, self.watch)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subfolders_of_an_authorized_folder_are_listed_through_its_session_without_a_prompt() {
    let mut harness = authorized(&[]).await;
    let root = harness.root.clone();
    let child = root.join("child");
    let grandchild = child.join("grandchild");
    harness.navigate(3, &child).await;
    let rows = harness.authorized_rows(3).await;
    assert_eq!(
        rows[0].native_path(),
        Some(child.join("inside.txt").as_path())
    );
    assert_eq!(rows[0].metadata.size, Some(7));
    harness.navigate(4, &grandchild).await;
    harness.authorized_rows(4).await;
    // The UI's Refresh of authorized rows is a navigation; a Refresh is served too.
    harness.navigate(5, &root).await;
    harness.authorized_rows(5).await;
    harness
        .commands
        .send(Command::Refresh {
            pane: PaneId::Left,
            navigation: NavigationId(6),
            path: child.clone(),
            sort: SortSpec::default(),
            scan: None,
        })
        .await
        .unwrap();
    harness.authorized_rows(6).await;
    {
        let calls = harness.calls();
        assert_eq!(calls.opened, std::slice::from_ref(&root), "one consent");
        assert_eq!(
            calls.listed,
            [
                child.clone(),
                grandchild.clone(),
                root.clone(),
                child.clone()
            ]
        );
        assert_eq!(calls.closed, 0);
    }
    let (calls, filesystem, _) = harness.shutdown().await;
    assert_eq!(calls.lock().unwrap().closed, 1, "shutdown ends the session");
    assert_eq!(
        *filesystem.0.lock().unwrap(),
        [root.clone(), child.clone(), grandchild, root, child],
        "each listing asks with ordinary rights first; the retry does not"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leaving_the_authorized_folder_ends_its_session() {
    let mut harness = authorized(&[]).await;
    let root = harness.root.clone();
    let parent = root.parent().unwrap().to_owned();
    // A re-sort of the authorized rows stays inside.
    harness
        .commands
        .send(Command::Resort {
            pane: PaneId::Left,
            navigation: NavigationId(3),
            path: root.clone(),
            sort: SortSpec::default(),
            source: None,
        })
        .await
        .unwrap();
    harness.authorized_rows(3).await;
    assert_eq!(harness.calls().closed, 0);
    // Backspace above the folder: listed with ordinary rights, which are denied here too.
    harness.navigate(4, &parent).await;
    harness.offered(4).await;
    assert_eq!(
        harness.calls().closed,
        1,
        "leaving the folder ends the session"
    );
    // Back inside: no session, so the denial offers authorization again.
    harness.navigate(5, &root.join("child")).await;
    harness.offered(5).await;
    assert_eq!(harness.calls().listed, std::slice::from_ref(&root));
    assert_eq!(harness.calls().opened, std::slice::from_ref(&root));
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_that_ended_offers_authorization_again() {
    let mut harness = authorized(&[]).await;
    let root = harness.root.clone();
    let child = root.join("child");
    harness.ended.store(true, Ordering::SeqCst);
    harness.navigate(3, &child).await;
    match harness.outcome(3).await {
        Outcome::Failed(error, Some(ElevationAvailability::Available)) => {
            // The original denial, as before sessions.
            assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
            assert_eq!(error.operation, "list");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(harness.calls().closed, 1, "the ended session was dropped");
    // The explicit retry asks again and opens a new session.
    harness
        .commands
        .send(Command::RetryDirectoryWithElevation {
            pane: PaneId::Left,
            denied_navigation: NavigationId(3),
            navigation: NavigationId(4),
        })
        .await
        .unwrap();
    match harness.outcome(4).await {
        Outcome::Listed(rows, _) => {
            assert_eq!(
                rows[0].native_path(),
                Some(child.join("visible.txt").as_path())
            );
        }
        other => panic!("{other:?}"),
    }
    harness.navigate(5, &child.join("deeper")).await;
    harness.authorized_rows(5).await;
    assert_eq!(harness.calls().opened, [root, child.clone()]);
    assert_eq!(harness.calls().listed, [child.join("deeper")]);
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_link_that_leads_outside_the_authorized_folder_is_refused_and_offered_on_its_own() {
    let mut harness = authorized(&["link"]).await;
    let root = harness.root.clone();
    harness.navigate(3, &root.join("link")).await;
    match harness.outcome(3).await {
        Outcome::Failed(error, Some(ElevationAvailability::Available)) => {
            assert!(error.is_outside_authorization(), "{error}");
            assert_eq!(error.path, root.join("link"));
        }
        other => panic!("{other:?}"),
    }
    // The session continues for the folders inside.
    harness.navigate(4, &root.join("child")).await;
    harness.authorized_rows(4).await;
    assert_eq!(harness.calls().closed, 0);
    assert_eq!(harness.calls().opened, [root]);
    harness.shutdown().await;
}

/// A session belongs to the pane whose retry opened it: the other pane, navigating into the
/// same folder or below it, is denied and offered its own authorization, never served
/// through that session; and the authorizing pane keeps its session meanwhile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_other_pane_never_lists_through_a_pane_session() {
    let mut harness = authorized(&[]).await;
    let root = harness.root.clone();
    for (navigation, path) in [(1, root.clone()), (2, root.join("child"))] {
        harness
            .commands
            .send(Command::Navigate {
                pane: PaneId::Right,
                navigation: NavigationId(navigation),
                path,
                sort: SortSpec::default(),
            })
            .await
            .unwrap();
        loop {
            match harness.next().await {
                Event::ListingAuthorized {
                    pane: PaneId::Right,
                    ..
                } => panic!("the right pane listed through the left pane's session"),
                Event::ListingFinished {
                    pane: PaneId::Right,
                    ..
                } => panic!("the right pane listed a denied folder"),
                Event::DirectoryElevationAvailable {
                    pane: PaneId::Right,
                    navigation: offer,
                    availability: ElevationAvailability::Available,
                    ..
                } if offer == NavigationId(navigation) => break,
                _ => {}
            }
        }
    }
    {
        let calls = harness.calls();
        assert!(calls.listed.is_empty(), "{:?}", calls.listed);
        assert_eq!(calls.opened, std::slice::from_ref(&root));
        assert_eq!(calls.closed, 0, "the left pane keeps its session");
    }
    harness.navigate(3, &root.join("child")).await;
    harness.authorized_rows(3).await;
    assert_eq!(harness.calls().listed, [root.join("child")]);
    harness.shutdown().await;
}

/// Rows listed through the session are a snapshot of a folder this process may not read:
/// the pane's watch neither subscribes nor checks them, as for the retry's own rows.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_listing_is_neither_watched_nor_checked() {
    let mut harness = authorized(&[]).await;
    let child = harness.root.join("child");
    harness.navigate(3, &child).await;
    let rows = harness.authorized_rows(3).await;
    harness
        .commands
        .send(Command::PaneCommitted(VisibleDirectory {
            pane: PaneId::Left,
            navigation: NavigationId(3),
            path: child.clone(),
            entries: rows,
        }))
        .await
        .unwrap();
    // A folder without notifications is checked about a second after acceptance.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2_500);
    while let Ok(Some(event)) = tokio::time::timeout_at(deadline, harness.events.recv()).await {
        match event {
            Event::DirectoryChanged { navigation, .. } => {
                panic!("rows of navigation {navigation:?} were checked with ordinary rights")
            }
            Event::ListingFallback { path, .. } => panic!("the pane moved to {path:?}"),
            _ => {}
        }
    }
    let (_, filesystem, watch) = harness.shutdown().await;
    assert!(
        watch.0.lock().unwrap().is_empty(),
        "a folder listed with authorization is never subscribed"
    );
    assert_eq!(filesystem.0.lock().unwrap().len(), 2, "no ordinary re-read");
}
