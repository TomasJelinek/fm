use midnight_core::{
    Cancellation, Command, DirectoryService, ElevatedDirectoryService, Entry, EntryId, EntryKind,
    EntryMetadata, Event, FsError, NavigationId, PaneId, SortSpec, SpaceInfo, StorageService,
    Volume, VolumeService,
};
use midnight_engine::Engine;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

struct Denied;
impl DirectoryService for Denied {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
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
    let engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        Arc::new(Denied),
        sender,
    )
    .with_elevated_directories(service.clone());
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
    let engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        Arc::new(Denied),
        sender,
    )
    .with_elevated_directories(service.clone());
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
