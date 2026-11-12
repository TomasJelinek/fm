use super::*;
use crate::{CommittedSnapshot, SnapshotState};
use midnight_core::{Entry, EntryId, EntryKind, FolderSize, NavigationId, PaneId, SearchRevision};
use std::{collections::HashMap, path::PathBuf, sync::Mutex, time::Duration};

fn query(pane: PaneId, revision: u64, text: &str) -> SearchRequest {
    let location = PathBuf::from(match pane {
        PaneId::Left => "left",
        PaneId::Right => "right",
    });
    SearchRequest {
        pane,
        navigation: NavigationId(1),
        revision: SearchRevision(revision),
        source: ["alpha", "beta"]
            .iter()
            .enumerate()
            .map(|(row, name)| {
                Entry::new(
                    EntryId(row as u64 + 1),
                    location.join(name),
                    EntryKind::File,
                )
            })
            .collect::<Vec<_>>()
            .into(),
        location,
        query: text.into(),
        filter: Default::default(),
    }
}

fn snapshot(request: &SearchRequest) -> CommittedSnapshot {
    CommittedSnapshot {
        navigation: request.navigation,
        path: request.location.clone(),
        entries: request.source.clone(),
        sizes: Arc::new(Mutex::new(HashMap::<PathBuf, FolderSize>::new())),
    }
}

fn sources(left: &SearchRequest, right: &SearchRequest) -> Snapshots {
    crate::test_snapshots([
        SnapshotState {
            tree_root: None,
            tree_notify: None,
            archive: None,
            network: None,
            network_navigation: None,
            archive_navigation: None,
            archive_sort_sizes: None,
            committed: Some(snapshot(left)),
            visible: Some(snapshot(left)),
            pending: None,
        },
        SnapshotState {
            tree_root: None,
            tree_notify: None,
            archive: None,
            network: None,
            network_navigation: None,
            archive_navigation: None,
            archive_sort_sizes: None,
            committed: Some(snapshot(right)),
            visible: Some(snapshot(right)),
            pending: None,
        },
    ])
}

#[test]
fn virtual_search_requires_the_exact_committed_snapshot_and_generation() {
    let mut request = query(PaneId::Left, 1, "a");
    let snapshots: Snapshots = crate::test_snapshots(Default::default());
    snapshots.lock().unwrap()[0].archive = Some((request.navigation, request.source.clone()));
    assert!(source_matches(&snapshots, &request));
    request.navigation = NavigationId(2);
    assert!(!source_matches(&snapshots, &request));
    request.navigation = NavigationId(1);
    request.source = request.source.to_vec().into();
    assert!(!source_matches(&snapshots, &request));
}

fn outbox(capacity: usize) -> (Outbox, tokio::sync::mpsc::Receiver<midnight_core::Event>) {
    let (sender, receiver) = tokio::sync::mpsc::channel(capacity);
    (
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        receiver,
    )
}

async fn stop(controller: Controller, tasks: &mut tokio::task::JoinSet<()>) {
    drop(controller);
    tokio::time::timeout(Duration::from_secs(4), async {
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .expect("search actors must stop without draining output");
}

async fn next(
    events: &mut tokio::sync::mpsc::Receiver<midnight_core::Event>,
) -> midnight_core::Event {
    tokio::time::timeout(Duration::from_secs(4), events.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn latest_query_wins_after_escape_and_output_backpressure() {
    let left = query(PaneId::Left, 1, "alpha");
    let right = query(PaneId::Right, 1, "alpha");
    let (outbox, mut events) = outbox(1);
    outbox
        .sender
        .send(midnight_core::Event::VolumesUpdated(vec![]))
        .await
        .unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    let controller = Controller::new(sources(&left, &right), outbox, &mut tasks);
    controller.query(left.clone());
    // Paused time advances only after the CPU worker and ready tasks yield: the
    // first completed result is now waiting behind the deliberately full outbox.
    tokio::time::sleep(Duration::from_secs(1)).await;
    controller.cancel(PaneId::Left, SearchRevision(2));
    let mut latest = left.clone();
    latest.revision = SearchRevision(3);
    latest.query = "beta".into();
    controller.query(latest.clone());
    controller.query(left);
    controller.cancel(PaneId::Left, SearchRevision(2));
    controller.query(SearchRequest {
        query: "alpha".into(),
        ..latest
    });
    assert!(matches!(
        next(&mut events).await,
        midnight_core::Event::VolumesUpdated(_)
    ));
    match next(&mut events).await {
        midnight_core::Event::SearchReady {
            revision: SearchRevision(3),
            result,
            ..
        } => {
            assert_eq!(result.matches.len(), 1);
            assert_eq!(result.source[result.matches[0].source_row].name, "beta");
        }
        event => panic!("obsolete search escaped its revision guard: {event:?}"),
    }
    stop(controller, &mut tasks).await;
    assert!(events.recv().await.is_none());
}

#[tokio::test]
async fn forged_source_location_navigation_and_refresh_are_rejected() {
    let left = query(PaneId::Left, 1, "alpha");
    let right = query(PaneId::Right, 1, "alpha");
    let snapshots = sources(&left, &right);
    let (outbox, mut events) = outbox(8);
    let mut tasks = tokio::task::JoinSet::new();
    let controller = Controller::new(snapshots.clone(), outbox, &mut tasks);
    for mutation in 0..4 {
        let mut forged = left.clone();
        forged.revision = SearchRevision(mutation + 1);
        match mutation {
            0 => forged.source = forged.source.to_vec().into(),
            1 => forged.location = "other".into(),
            2 => forged.navigation = NavigationId(99),
            _ => snapshots.lock().unwrap()[0].pending = Some(NavigationId(2)),
        }
        controller.query(forged);
        match next(&mut events).await {
            midnight_core::Event::SearchFailed { error, .. } => {
                assert_eq!(error.kind, io::ErrorKind::InvalidInput)
            }
            event => panic!("forged/staging source was accepted: {event:?}"),
        }
    }
    stop(controller, &mut tasks).await;
}

#[tokio::test(start_paused = true)]
async fn refresh_replaces_the_source_without_publishing_older_matches() {
    let left = query(PaneId::Left, 1, "alpha");
    let right = query(PaneId::Right, 1, "alpha");
    let snapshots = sources(&left, &right);
    let (outbox, mut events) = outbox(1);
    outbox
        .sender
        .send(midnight_core::Event::VolumesUpdated(vec![]))
        .await
        .unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    let controller = Controller::new(snapshots.clone(), outbox, &mut tasks);
    controller.query(left.clone());
    tokio::time::sleep(Duration::from_secs(1)).await;
    controller.invalidate(PaneId::Left);
    let mut current = query(PaneId::Left, 2, "beta");
    current.navigation = NavigationId(2);
    snapshots.lock().unwrap()[0] = SnapshotState {
        tree_root: None,
        tree_notify: None,
        archive: None,
        network: None,
        network_navigation: None,
        archive_navigation: None,
        archive_sort_sizes: None,
        committed: Some(snapshot(&current)),
        visible: None,
        pending: None,
    };
    controller.query(current.clone());
    assert!(matches!(
        next(&mut events).await,
        midnight_core::Event::VolumesUpdated(_)
    ));
    match next(&mut events).await {
        midnight_core::Event::SearchReady {
            navigation: NavigationId(2),
            result,
            ..
        } => assert!(Arc::ptr_eq(&result.source, &current.source)),
        event => panic!("old source survived refresh: {event:?}"),
    }
    stop(controller, &mut tasks).await;
    assert!(events.recv().await.is_none());
}

#[tokio::test]
async fn cancelled_left_pane_cannot_block_right_results_or_shutdown_on_full_output() {
    let left = query(PaneId::Left, 1, "alpha");
    let right = query(PaneId::Right, 1, "beta");
    let (outbox, mut events) = outbox(1);
    outbox
        .sender
        .send(midnight_core::Event::VolumesUpdated(vec![]))
        .await
        .unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    let controller = Controller::new(sources(&left, &right), outbox, &mut tasks);
    controller.query(left.clone());
    controller.cancel(PaneId::Left, SearchRevision(2));
    controller.query(right);
    assert!(matches!(
        next(&mut events).await,
        midnight_core::Event::VolumesUpdated(_)
    ));
    assert!(matches!(
        next(&mut events).await,
        midnight_core::Event::SearchReady {
            pane: PaneId::Right,
            ..
        }
    ));
    let mut latest = left;
    latest.revision = SearchRevision(3);
    controller.query(latest);
    // Leave any resulting output queued; stopping cannot require its consumption.
    stop(controller, &mut tasks).await;
}

#[tokio::test(start_paused = true)]
async fn idle_search_actors_leave_paused_runtime_timers_free_to_advance() {
    let left = query(PaneId::Left, 1, "alpha");
    let right = query(PaneId::Right, 1, "alpha");
    let (outbox, _events) = outbox(1);
    let mut tasks = tokio::task::JoinSet::new();
    let controller = Controller::new(sources(&left, &right), outbox, &mut tasks);
    let before = tokio::time::Instant::now();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(before.elapsed(), Duration::from_secs(5));
    stop(controller, &mut tasks).await;
}
