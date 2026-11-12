use midnight_core::{
    Cancellation, Command, DirectoryService, ElevatedDirectoryService, Entry, EntryId, EntryKind,
    EntryMetadata, Event, FsError, NavigationFallbackService, NavigationId, PaneId, SortSpec,
    SpaceInfo, StorageService, Volume, VolumeService,
};
use midnight_engine::Engine;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

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
#[derive(Default)]
struct Authorized(Mutex<Vec<PathBuf>>);
impl ElevatedDirectoryService for Authorized {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.0.lock().unwrap().push(path.to_owned());
        if !cancel.is_cancelled() {
            let mut entry = Entry::new(EntryId(1), path.join("visible.txt"), EntryKind::File);
            entry.metadata.size = Some(19);
            emit(entry);
        }
        Ok(())
    }
}
async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_explicit_receipt_retries_captured_path_once_and_preserves_complete_metadata() {
    let path = std::env::current_dir().unwrap().join("denied-unicode-中文");
    let (commands, receiver) = mpsc::channel(16);
    let (sender, mut events) = mpsc::channel(32);
    let service = Arc::new(Authorized::default());
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        Arc::new(Denied::default()),
        sender,
    );
    engine
        .configure_elevated_directories()
        .unwrap()
        .install(service.clone());
    let task = tokio::spawn(engine.run(receiver));
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: path.clone(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    while !matches!(
        next(&mut events).await,
        Event::DirectoryElevationAvailable {
            navigation: NavigationId(1),
            ..
        }
    ) {}
    assert!(
        service.0.lock().unwrap().is_empty(),
        "denial must not automatically prompt"
    );
    commands
        .send(Command::RetryDirectoryWithElevation {
            pane: PaneId::Left,
            denied_navigation: NavigationId(1),
            navigation: NavigationId(2),
        })
        .await
        .unwrap();
    loop {
        match next(&mut events).await {
            Event::ListingFinished {
                navigation: NavigationId(2),
                entries,
                ..
            } => {
                assert_eq!(
                    entries[0].native_path(),
                    Some(path.join("visible.txt").as_path())
                );
                assert_eq!(entries[0].metadata.size, Some(19));
                break;
            }
            Event::ListingFailed { error, .. } => panic!("{error}"),
            _ => {}
        }
    }
    commands
        .send(Command::RetryDirectoryWithElevation {
            pane: PaneId::Left,
            denied_navigation: NavigationId(1),
            navigation: NavigationId(3),
        })
        .await
        .unwrap();
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
    assert_eq!(*service.0.lock().unwrap(), vec![path]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_refresh_denial_never_offers_authorization() {
    let path = std::env::current_dir().unwrap().join("denied");
    let (commands, receiver) = mpsc::channel(16);
    let (sender, mut events) = mpsc::channel(32);
    let service = Arc::new(Authorized::default());
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        Arc::new(Denied::default()),
        sender,
    );
    engine
        .configure_elevated_directories()
        .unwrap()
        .install(service.clone());
    let task = tokio::spawn(engine.run(receiver));
    commands
        .send(Command::Refresh {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path,
            sort: SortSpec::default(),
            scan: None,
        })
        .await
        .unwrap();
    loop {
        match next(&mut events).await {
            Event::DirectoryElevationAvailable { .. } => {
                panic!("background refresh cannot offer elevation")
            }
            Event::ListingFailed { .. } => break,
            _ => {}
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, Event::DirectoryElevationAvailable { .. }));
    }
    assert!(service.0.lock().unwrap().is_empty());
}

struct FailedAuthorization {
    kind: io::ErrorKind,
    paths: Mutex<Vec<PathBuf>>,
}
impl ElevatedDirectoryService for FailedAuthorization {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.paths.lock().unwrap().push(path.to_owned());
        Err(FsError::from_io(
            "authorized directory listing",
            path,
            io::Error::new(self.kind, "controlled authorization failure"),
        ))
    }
}

struct Fallback {
    root: PathBuf,
    calls: Mutex<usize>,
}
impl NavigationFallbackService for Fallback {
    fn navigation_fallback_root(&self) -> Result<PathBuf, FsError> {
        *self.calls.lock().unwrap() += 1;
        Ok(self.root.clone())
    }
}

async fn assert_failed_authorization_retry(kind: io::ErrorKind) {
    let path = std::env::current_dir().unwrap().join("denied-unicode-中文");
    let (commands, receiver) = mpsc::channel(16);
    let (sender, mut events) = mpsc::channel(32);
    let filesystem = Arc::new(Denied::default());
    let service = Arc::new(FailedAuthorization {
        kind,
        paths: Mutex::new(Vec::new()),
    });
    let fallback = Arc::new(Fallback {
        root: path.ancestors().last().unwrap().to_owned(),
        calls: Mutex::new(0),
    });
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        filesystem.clone(),
        sender,
    );
    engine
        .configure_elevated_directories()
        .unwrap()
        .install(service.clone());
    engine
        .configure_navigation_fallback()
        .unwrap()
        .install(fallback.clone());
    let task = tokio::spawn(engine.run(receiver));
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: path.clone(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    loop {
        if let Event::DirectoryElevationAvailable {
            pane,
            navigation,
            path: offered_path,
        } = next(&mut events).await
        {
            assert_eq!(pane, PaneId::Left);
            assert_eq!(navigation, NavigationId(1));
            assert_eq!(offered_path, path);
            break;
        }
    }
    assert!(service.paths.lock().unwrap().is_empty());
    commands
        .send(Command::RetryDirectoryWithElevation {
            pane: PaneId::Left,
            denied_navigation: NavigationId(1),
            navigation: NavigationId(2),
        })
        .await
        .unwrap();
    let mut retry_events = Vec::new();
    loop {
        let event = next(&mut events).await;
        let terminal = matches!(
            &event,
            Event::ListingFailed { .. }
                | Event::ListingFinished { .. }
                | Event::ListingFallback { .. }
        );
        retry_events.push(event);
        if terminal {
            break;
        }
    }
    // Neither the consumed receipt nor the failed retry grants another prompt.
    for (denied_navigation, navigation) in [
        (NavigationId(1), NavigationId(3)),
        (NavigationId(2), NavigationId(4)),
    ] {
        commands
            .send(Command::RetryDirectoryWithElevation {
                pane: PaneId::Left,
                denied_navigation,
                navigation,
            })
            .await
            .unwrap();
    }
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    while let Ok(event) = events.try_recv() {
        retry_events.push(event);
    }
    let mut started = false;
    let mut failed = false;
    for event in retry_events {
        match event {
            Event::ListingStarted {
                pane,
                navigation,
                path: started_path,
            } => {
                assert!(!started, "authorization retry started more than once");
                assert_eq!(pane, PaneId::Left);
                assert_eq!(navigation, NavigationId(2));
                assert_eq!(started_path, path);
                started = true;
            }
            Event::ListingFailed {
                pane,
                navigation,
                error,
            } => {
                assert!(started, "failure must follow the retry's listing start");
                assert!(!failed, "authorization retry failed more than once");
                assert_eq!(pane, PaneId::Left);
                assert_eq!(navigation, NavigationId(2));
                assert_eq!(error.operation, "authorized directory listing");
                assert_eq!(error.path, path);
                assert_eq!(error.kind, kind);
                assert_eq!(error.message, "controlled authorization failure");
                failed = true;
            }
            Event::ListingFinished { .. } => panic!("failed authorization published success"),
            Event::ListingFallback { .. } => panic!("failed authorization changed destination"),
            Event::DirectoryElevationAvailable { .. } => {
                panic!("failed authorization offered another prompt")
            }
            Event::EntriesBatch { .. } | Event::MetadataBatch { .. } => {
                panic!("failed authorization published listing data")
            }
            _ => {}
        }
    }
    assert!(started, "authorization retry did not start");
    assert!(failed, "authorization retry did not report its failure");
    assert_eq!(*service.paths.lock().unwrap(), vec![path.clone()]);
    assert_eq!(*fallback.calls.lock().unwrap(), 0);
    assert_eq!(
        *filesystem.0.lock().unwrap(),
        vec![path],
        "authorization failure must not retry with ordinary privileges"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn still_denied_authorization_retry_reports_failure_without_fallback() {
    assert_failed_authorization_retry(io::ErrorKind::PermissionDenied).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupted_authorization_retry_reports_failure_without_fallback() {
    assert_failed_authorization_retry(io::ErrorKind::Interrupted).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transport_failed_authorization_retry_reports_failure_without_fallback() {
    assert_failed_authorization_retry(io::ErrorKind::Other).await;
}
