use midnight_core::*;
use midnight_engine::Engine;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Default)]
struct Files {
    reads: AtomicUsize,
    metadata: AtomicUsize,
}

impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        for (id, name) in [(1, "Straße.txt"), (2, "unrelated.txt")] {
            if !emit(Entry::new(EntryId(id), path.join(name), EntryKind::File)) {
                break;
            }
        }
        Ok(())
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        self.metadata.fetch_add(1, Ordering::SeqCst);
        Ok(EntryMetadata {
            size: Some(7),
            ..Default::default()
        })
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for Files {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            available: 1,
            total: 2,
        })
    }
}

#[tokio::test]
async fn engine_search_uses_the_exact_accepted_snapshot_without_filesystem_work() {
    let files = Arc::new(Files::default());
    let (commands, receiver) = mpsc::channel(16);
    let (sender, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            files.clone(),
            sender,
        )
        .run(receiver),
    );
    let location = PathBuf::from("folder");
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: location.clone(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let source = loop {
        let event = tokio::time::timeout(Duration::from_secs(4), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let Event::ListingFinished { entries, .. } = event {
            break entries;
        }
    };
    assert_eq!(files.reads.load(Ordering::SeqCst), 1);
    assert_eq!(files.metadata.load(Ordering::SeqCst), 2);
    commands
        .send(Command::Search(SearchRequest {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            revision: SearchRevision(1),
            location,
            source: source.clone(),
            query: "STRASS".into(),
            filter: Default::default(),
        }))
        .await
        .unwrap();
    let result = loop {
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("an accepted in-memory query must produce a result")
            .unwrap();
        if let Event::SearchReady {
            revision: SearchRevision(1),
            result,
            ..
        } = event
        {
            break result;
        }
    };
    assert!(Arc::ptr_eq(&result.source, &source));
    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.source[result.matches[0].source_row].id, EntryId(1));
    assert_eq!(result.matches[0].display_prefix_end, 6);
    assert_eq!(files.reads.load(Ordering::SeqCst), 1);
    assert_eq!(files.metadata.load(Ordering::SeqCst), 2);
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    engine.await.unwrap();
}
