use midnight_core::*;
use midnight_engine::Engine;
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::ThreadId,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc;

struct Fixture {
    complete: bool,
    stats: AtomicUsize,
}

fn timestamp() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn native_datetime() -> LocalizedDateTime {
    LocalizedDateTime {
        date: "14. 11. 2023".into(),
        time: "23:13:20".into(),
    }
}

fn provider_datetime() -> LocalizedDateTime {
    LocalizedDateTime {
        date: "Provider date".into(),
        time: "Provider time".into(),
    }
}

fn metadata(path: &Path) -> EntryMetadata {
    EntryMetadata {
        size: Some(17),
        modified: Some(timestamp()),
        localized_modified: (path.file_name().unwrap() == "provided").then(provider_datetime),
        ..Default::default()
    }
}

impl DirectoryService for Fixture {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        for (id, name) in ["native", "provided"].into_iter().enumerate() {
            let mut entry = Entry::new(EntryId(id as u64), path.join(name), EntryKind::File);
            if self.complete {
                entry.metadata = metadata(&entry.path);
            }
            if !emit(entry) {
                break;
            }
        }
        Ok(())
    }

    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        self.complete
    }

    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        self.stats.fetch_add(1, Ordering::Relaxed);
        Ok(metadata(path))
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
    calls: Mutex<Vec<(SystemTime, ThreadId)>>,
}

impl LocaleService for Locale {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(Default::default())
    }

    fn format_datetime(&self, value: SystemTime) -> Option<LocalizedDateTime> {
        self.calls
            .lock()
            .unwrap()
            .push((value, std::thread::current().id()));
        Some(native_datetime())
    }
}

fn assert_localized(entries: &[(EntryId, EntryMetadata)]) {
    assert_eq!(entries.len(), 2);
    let value = |id| {
        &entries
            .iter()
            .find(|entry| entry.0 == EntryId(id))
            .unwrap()
            .1
    };
    assert_eq!(value(0).modified, Some(timestamp()));
    assert_eq!(value(0).localized_modified, Some(native_datetime()));
    assert_eq!(value(1).localized_modified, Some(provider_datetime()));
}

async fn verify_listing(complete: bool) {
    let filesystem = Arc::new(Fixture {
        complete,
        stats: AtomicUsize::new(0),
    });
    let locale = Arc::new(Locale::default());
    let caller = std::thread::current().id();
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .with_locale(locale.clone())
        .run(receiver),
    );
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: "active".into(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let (streamed, enriched, finished) = tokio::time::timeout(Duration::from_secs(3), async {
        let mut streamed = Vec::new();
        let mut enriched = Vec::new();
        loop {
            match events.recv().await.unwrap() {
                Event::EntriesBatch { entries, .. } => {
                    streamed.extend(entries.into_iter().map(|entry| (entry.id, entry.metadata)));
                }
                Event::MetadataBatch { entries, .. } => enriched.extend(entries),
                Event::ListingFinished { entries, .. } => {
                    return (
                        streamed,
                        enriched,
                        entries
                            .iter()
                            .map(|entry| (entry.id, entry.metadata.clone()))
                            .collect::<Vec<_>>(),
                    );
                }
                Event::ListingFailed { error, .. } => panic!("unexpected listing error: {error}"),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();

    if complete {
        assert_localized(&streamed);
        assert!(
            enriched.is_empty(),
            "complete metadata needs no stat update"
        );
        assert_eq!(filesystem.stats.load(Ordering::Relaxed), 0);
    } else {
        assert_eq!(streamed.len(), 2);
        assert!(streamed.iter().all(|entry| entry.1.modified.is_none()));
        assert_localized(&enriched);
        assert_eq!(filesystem.stats.load(Ordering::Relaxed), 2);
    }
    assert_localized(&finished);
    let calls = locale.calls.lock().unwrap();
    assert_eq!(
        calls.len(),
        1,
        "provider localization must remain untouched"
    );
    assert_eq!(calls[0].0, timestamp());
    assert_ne!(
        calls[0].1, caller,
        "date formatting belongs on a backend worker"
    );
}

#[tokio::test]
async fn complete_listing_localizes_timestamps_without_reopening_files() {
    verify_listing(true).await;
}

#[tokio::test]
async fn stat_fallback_localizes_timestamps_before_metadata_publication() {
    verify_listing(false).await;
}
