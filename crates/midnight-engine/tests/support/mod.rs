use midnight_core::{Command, Entry, EntryId, Event, NavigationId, PaneId, SortSpec};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc;

// Catch path conversion, dropped batches, stale generation results, and metadata
// associated with the wrong identity across the real provider -> engine boundary.
pub async fn navigate(
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    pane: PaneId,
    generation: u64,
    path: &Path,
) -> Arc<[Entry]> {
    let navigation = NavigationId(generation);
    commands
        .send(Command::Navigate {
            pane,
            navigation,
            path: path.to_owned(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut started = false;
        let mut streamed = HashSet::new();
        let mut metadata = HashMap::<EntryId, u64>::new();
        loop {
            match events.recv().await.expect("engine event channel closed") {
                Event::ListingStarted {
                    pane: actual_pane,
                    navigation: actual,
                    path: actual_path,
                } if actual_pane == pane && actual == navigation => {
                    assert_eq!(actual_path, path);
                    started = true;
                }
                Event::EntriesBatch {
                    pane: actual_pane,
                    navigation: actual,
                    entries,
                } if actual_pane == pane && actual == navigation => {
                    for entry in entries {
                        assert!(streamed.insert((entry.id, entry.path)));
                    }
                }
                Event::MetadataBatch {
                    pane: actual_pane,
                    navigation: actual,
                    entries,
                } if actual_pane == pane && actual == navigation => {
                    for (id, value) in entries {
                        assert!(value.modified.is_some());
                        if let Some(size) = value.size {
                            metadata.insert(id, size);
                        }
                    }
                }
                Event::ListingFinished {
                    pane: actual_pane,
                    navigation: actual,
                    entries,
                    index,
                } if actual_pane == pane && actual == navigation => {
                    assert!(started);
                    assert_eq!(streamed.len(), entries.len());
                    for (position, entry) in entries.iter().enumerate() {
                        assert!(streamed.contains(&(entry.id, entry.path.clone())));
                        assert_eq!(index.by_id.get(&entry.id), Some(&position));
                        assert_eq!(index.by_path.get(&entry.path), Some(&entry.id));
                        if let Some(size) = entry.metadata.size {
                            assert_eq!(metadata.get(&entry.id), Some(&size));
                        }
                    }
                    return entries;
                }
                Event::ListingFailed {
                    pane: actual_pane,
                    navigation: actual,
                    error,
                } if actual_pane == pane && actual == navigation => {
                    panic!("Unicode navigation failed: {error}")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("Unicode navigation did not complete")
}
