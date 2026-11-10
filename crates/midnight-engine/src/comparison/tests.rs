use super::*;
use std::{
    path::Path,
    time::{Duration, UNIX_EPOCH},
};

#[cfg(unix)]
#[path = "tests/unix.rs"]
mod unix;
#[cfg(windows)]
#[path = "tests/windows.rs"]
mod windows;

fn entry(id: u64, name: &str, size: Option<u64>, seconds: u64) -> Entry {
    let mut entry = Entry::new(EntryId(id), Path::new("/left").join(name), EntryKind::File);
    entry.metadata.size = size;
    entry.metadata.modified = Some(UNIX_EPOCH + Duration::from_secs(seconds));
    entry
}

fn request(left: Vec<Entry>, mut right: Vec<Entry>) -> Arc<CompareRequest> {
    for entry in &mut right {
        entry.path = Path::new("/right").join(&entry.name);
    }
    let sides = [(PaneId::Left, left), (PaneId::Right, right)].map(|(pane, entries)| {
        let view = Arc::new(DirectoryIndex::from_entries(&entries));
        CompareSide {
            source: VisibleDirectory {
                pane,
                navigation: NavigationId(1),
                path: Path::new(if pane == PaneId::Left {
                    "/left"
                } else {
                    "/right"
                })
                .into(),
                entries: entries.into(),
            },
            view,
            view_revision: 0,
            selection_revision: SelectionRevision(0),
        }
    });
    Arc::new(CompareRequest {
        id: CompareId(1),
        sides,
        options: CompareOptions::default(),
    })
}

#[test]
fn size_time_and_missing_names_select_the_correct_sides() {
    let request = request(
        vec![
            entry(1, "both", Some(20), 10),
            entry(2, "only-left", Some(1), 1),
            entry(3, "equal", Some(1), 10),
        ],
        vec![
            entry(1, "both", Some(10), 20),
            entry(2, "only-right", Some(1), 1),
            entry(3, "equal", Some(1), 12),
        ],
    );
    let result = compute(request, &|| false).unwrap();
    assert_eq!(result.selections[0].len(), 2);
    assert_eq!(result.selections[1].len(), 2);
    assert!(!result.selections[0].contains_path(Path::new("/left/equal")));
    assert_eq!(result.unknown, 0);
}

#[test]
fn unknown_metadata_and_ambiguous_folded_names_are_never_equal() {
    let mut request = request(
        vec![
            entry(1, "A", Some(1), 1),
            entry(2, "a", Some(1), 1),
            entry(3, "unknown", None, 1),
        ],
        vec![entry(1, "a", Some(1), 1), entry(3, "unknown", Some(1), 1)],
    );
    Arc::make_mut(&mut request).options.case_sensitive = false;
    let result = compute(request, &|| false).unwrap();
    assert_eq!(result.ambiguous, 1);
    assert_eq!(result.unknown, 1);
    assert_eq!(result.selections[0].len(), 3);
    assert_eq!(result.selections[1].len(), 2);
}

#[test]
fn cancelled_comparison_never_returns_a_partial_selection() {
    let request = request(vec![entry(1, "a", Some(1), 1)], vec![]);
    assert_eq!(
        compute(request, &|| true).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
}

#[test]
fn whole_second_tolerance_includes_boundary_before_epoch() {
    let mut left = entry(1, "date", Some(1), 0);
    let mut right = left.clone();
    left.metadata.modified = Some(UNIX_EPOCH - Duration::from_millis(2100));
    right.metadata.modified = Some(UNIX_EPOCH - Duration::from_millis(100));
    let result = compute(request(vec![left.clone()], vec![right.clone()]), &|| false).unwrap();
    assert!(
        result
            .selections
            .iter()
            .all(|selection| selection.is_empty())
    );
    right.metadata.modified = Some(UNIX_EPOCH + Duration::from_millis(100));
    let result = compute(request(vec![left], vec![right]), &|| false).unwrap();
    assert_eq!(result.selections[0].len(), 0);
    assert_eq!(result.selections[1].len(), 1);
}

#[test]
fn directory_option_is_unmatched_only_and_missing_requested_times_are_selected() {
    let mut folder = entry(1, "folder", None, 0);
    folder.kind = EntryKind::Directory;
    let mut file = entry(2, "unknown-time", Some(1), 0);
    file.metadata.modified = None;
    let mut capture = request(
        vec![folder, file],
        vec![entry(2, "unknown-time", Some(1), 0)],
    );
    let result = compute(capture.clone(), &|| false).unwrap();
    assert_eq!(result.selections[0].len(), 1);
    assert_eq!(result.unknown, 1);
    Arc::make_mut(&mut capture).options.unmatched_directories = true;
    assert_eq!(compute(capture, &|| false).unwrap().selections[0].len(), 2);
}

fn sources(request: &CompareRequest) -> Snapshots {
    use crate::{CommittedSnapshot, SnapshotState};
    use std::sync::Mutex;
    Arc::new(Mutex::new(request.sides.each_ref().map(|side| {
        let snapshot = CommittedSnapshot {
            navigation: side.source.navigation,
            path: side.source.path.clone(),
            entries: side.source.entries.clone(),
            sizes: Arc::new(Mutex::new(Default::default())),
        };
        SnapshotState {
            tree_root: None,
            tree_notify: None,
            archive: None,
            network: None,
            network_navigation: None,
            archive_navigation: None,
            archive_sort_sizes: None,
            committed: Some(snapshot.clone()),
            visible: Some(snapshot),
            pending: None,
        }
    })))
}

#[tokio::test(start_paused = true)]
async fn cancel_and_newer_comparison_replace_a_result_waiting_for_output_capacity() {
    let capture = request(vec![entry(1, "left", Some(1), 1)], vec![]);
    let (sender, mut events) = tokio::sync::mpsc::channel(1);
    sender.send(Event::VolumesUpdated(vec![])).await.unwrap();
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        sources(&capture),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    controller.compare((*capture).clone());
    tokio::time::sleep(Duration::from_secs(1)).await;
    controller.cancel(CompareId(2));
    let mut latest = (*capture).clone();
    latest.id = CompareId(3);
    controller.compare(latest);
    assert!(matches!(
        events.recv().await,
        Some(Event::VolumesUpdated(_))
    ));
    let event = tokio::time::timeout(Duration::from_secs(4), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        Event::DirectoriesCompared {
            id: CompareId(3),
            result: Ok(_)
        }
    ));
    drop(controller);
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    assert!(events.recv().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn either_source_refresh_discards_a_completed_result_under_backpressure() {
    let capture = request(vec![entry(1, "left", Some(1), 1)], vec![]);
    let snapshots = sources(&capture);
    let (sender, mut events) = tokio::sync::mpsc::channel(1);
    sender.send(Event::VolumesUpdated(vec![])).await.unwrap();
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        snapshots.clone(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    controller.compare((*capture).clone());
    tokio::time::sleep(Duration::from_secs(1)).await;
    snapshots.lock().unwrap()[1].pending = Some(NavigationId(2));
    controller.invalidate(PaneId::Right);
    assert!(matches!(
        events.recv().await,
        Some(Event::VolumesUpdated(_))
    ));
    drop(controller);
    tokio::time::timeout(Duration::from_secs(4), async {
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(events.recv().await.is_none());
}
