use midnight_core::*;
use midnight_engine::Engine;
use std::{
    ffi::OsStr,
    io,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Default)]
struct Fixture {
    reads: AtomicUsize,
}
impl DirectoryService for Fixture {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        for (id, name) in ["ch", "i", "h"].into_iter().enumerate() {
            if !emit(Entry::new(
                EntryId(id as u64),
                path.join(name),
                EntryKind::File,
            )) {
                break;
            }
        }
        Ok(())
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        Ok(EntryMetadata {
            size: Some(17),
            ..Default::default()
        })
    }
}
impl VolumeService for Fixture {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for Fixture {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            available: 0,
            total: 0,
        })
    }
}

#[derive(Default)]
struct Locale {
    calls: Mutex<Vec<std::thread::ThreadId>>,
    fail: bool,
}
impl LocaleService for Locale {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(Default::default())
    }
    fn collation_key(&self, name: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
        self.calls.lock().unwrap().push(std::thread::current().id());
        if self.fail {
            return Err(FsError::from_io(
                "native collation",
                Path::new(""),
                io::Error::other("injected native failure"),
            ));
        }
        Ok(Some(vec![match name.to_str().unwrap() {
            "h" => 1,
            "ch" => 2,
            "i" => 3,
            _ => 9,
        }]))
    }
}
async fn listing(events: &mut mpsc::Receiver<Event>, navigation: u64) -> Arc<[Entry]> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match events.recv().await.unwrap() {
                Event::ListingFinished {
                    navigation: actual,
                    entries,
                    ..
                } if actual == NavigationId(navigation) => return entries,
                Event::ListingFailed { error, .. } => panic!("unexpected listing error: {error}"),
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listing_uses_locale_on_a_worker_and_resort_reuses_keys_without_native_calls() {
    let provider = Arc::new(Fixture::default());
    let locale = Arc::new(Locale::default());
    let caller = std::thread::current().id();
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        provider.clone(),
        sender,
    );
    engine.configure_locale().unwrap().install(locale.clone());
    let engine = tokio::spawn(engine.run(receiver));
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: "active".into(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let entries = listing(&mut events, 1).await;
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.name.to_str().unwrap())
            .collect::<Vec<_>>(),
        ["h", "ch", "i"]
    );
    assert_eq!(locale.calls.lock().unwrap().len(), 3);
    assert!(locale.calls.lock().unwrap().iter().all(|id| *id != caller));
    commands
        .send(Command::Resort {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: "active".into(),
            sort: SortSpec {
                column: SortColumn::Size,
                descending: true,
            },
            source: Some(VisibleDirectory {
                pane: PaneId::Left,
                navigation: NavigationId(1),
                path: "active".into(),
                entries,
            }),
        })
        .await
        .unwrap();
    let entries = listing(&mut events, 2).await;
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.name.to_str().unwrap())
            .collect::<Vec<_>>(),
        ["h", "ch", "i"]
    );
    assert_eq!(locale.calls.lock().unwrap().len(), 3);
    assert_eq!(provider.reads.load(Ordering::Relaxed), 1);
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_collation_errors_report_listing_failure_without_publishing_wrong_order() {
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        Arc::new(Fixture::default()),
        sender,
    );
    engine.configure_locale().unwrap().install(Arc::new(Locale {
        fail: true,
        ..Default::default()
    }));
    let engine = tokio::spawn(engine.run(receiver));
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: "active".into(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match events.recv().await.unwrap() {
                Event::ListingFailed { error, .. } => {
                    assert!(error.message.contains("injected native failure"));
                    break;
                }
                Event::ListingFinished { .. } => {
                    panic!("native collation failure cannot claim a locale-sorted listing")
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}
