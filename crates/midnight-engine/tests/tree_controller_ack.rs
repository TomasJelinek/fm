//! Publication acknowledgement and visible row actions across background probes.
use midnight_core::*;
use midnight_engine::Engine;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;

async fn tree(commands: &mpsc::Sender<Command>, command: TreeCommand) {
    commands
        .send(Command::Tree(Box::new(command)))
        .await
        .unwrap();
}
async fn event(events: &mut mpsc::Receiver<Event>) -> TreeEvent {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Event::Tree(event)) = events.recv().await {
                return *event;
            }
        }
    })
    .await
    .expect("tree response")
}
async fn publication(events: &mut mpsc::Receiver<Event>) -> TreePublication {
    match event(events).await {
        TreeEvent::Published(publication) => publication,
        event => panic!("expected publication: {event:?}"),
    }
}
async fn start(
    path: &Path,
) -> (
    mpsc::Sender<Command>,
    mpsc::Receiver<Event>,
    tokio::task::JoinHandle<()>,
    TreePublication,
) {
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().unwrap(),
            Arc::new(midnight_platform::LocalFileSystem::new(None)),
            send,
        )
        .run(receive),
    );
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: path.to_owned(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let root = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Event::ListingFinished {
                pane,
                navigation,
                entries,
                index,
            }) = events.recv().await
            {
                break TreeRootSnapshot {
                    pane,
                    navigation,
                    entries,
                    index,
                };
            }
        }
    })
    .await
    .unwrap();
    tree(
        &commands,
        TreeCommand::Enable {
            root,
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        },
    )
    .await;
    let view = publication(&mut events).await;
    (commands, events, engine, view)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acknowledged_row_navigation_survives_unacknowledged_probe_publication() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("drivers");
    std::fs::create_dir(&target).unwrap();
    std::fs::create_dir(target.join("DriverData")).unwrap();
    let (commands, mut events, engine, visible) = start(directory.path()).await;
    let row = visible.view.rows[0].id;
    tree(&commands, TreeCommand::Commit(visible.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Viewport {
            rows: TreeViewport::new(&visible.view, vec![row]).unwrap(),
            view: visible.view.clone(),
        },
    )
    .await;
    let loading = publication(&mut events).await;
    assert_eq!(loading.view.rows[0].disclosure, TreeDisclosure::Loading);
    // The user still sees the acknowledged view while a newer publication waits
    // in the UI event queue. Exact occurrence authority must survive that delay.
    tree(
        &commands,
        TreeCommand::Navigate {
            view: visible.view.clone(),
            request: TreeRequestId(1),
            row,
            navigation: NavigationId(2),
        },
    )
    .await;
    let received = loop {
        let received = event(&mut events).await;
        if !matches!(received, TreeEvent::Published(_)) {
            break received;
        }
    };
    assert!(
        matches!(received, TreeEvent::Navigated { target: TreeNavigationTarget::Native(ref path), .. } if path == &target),
        "{received:?}"
    );
    tree(&commands, TreeCommand::Commit(loading.view)).await;
    let completed = publication(&mut events).await;
    assert_eq!(completed.view.rows[0].disclosure, TreeDisclosure::Collapsed);
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probe_publications_wait_for_ack_without_blocking_navigation() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["drivers", "other"] {
        let path = directory.path().join(name);
        std::fs::create_dir(&path).unwrap();
        std::fs::create_dir(path.join("child")).unwrap();
    }
    let (commands, mut events, engine, visible) = start(directory.path()).await;
    tree(&commands, TreeCommand::Commit(visible.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Viewport {
            rows: TreeViewport::new(
                &visible.view,
                visible.view.rows.iter().map(|row| row.id).collect(),
            )
            .unwrap(),
            view: visible.view.clone(),
        },
    )
    .await;
    let loading = publication(&mut events).await;
    // No second publication may invalidate the unacknowledged one. A navigation
    // command is an engine/model barrier and must still be admitted and serviced.
    tree(
        &commands,
        TreeCommand::Navigate {
            view: visible.view,
            request: TreeRequestId(1),
            row: loading.view.rows[1].id,
            navigation: NavigationId(2),
        },
    )
    .await;
    let received = event(&mut events).await;
    assert!(
        matches!(received, TreeEvent::Navigated { .. }),
        "publication overran acknowledgement: {received:?}"
    );
    tree(&commands, TreeCommand::Commit(loading.view)).await;
    let next = publication(&mut events).await;
    assert!(next.view.stamp.revision.0 > 1);
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acknowledged_row_expansion_survives_an_unrelated_probe_publication() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["drivers", "other"] {
        let path = directory.path().join(name);
        std::fs::create_dir(&path).unwrap();
        std::fs::create_dir(path.join("child")).unwrap();
    }
    let (commands, mut events, engine, visible) = start(directory.path()).await;
    let drivers = visible
        .view
        .rows
        .iter()
        .find(|row| row.entry.name == "drivers")
        .unwrap()
        .id;
    let other = visible
        .view
        .rows
        .iter()
        .find(|row| row.entry.name == "other")
        .unwrap()
        .id;
    tree(&commands, TreeCommand::Commit(visible.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Viewport {
            rows: TreeViewport::new(&visible.view, vec![other]).unwrap(),
            view: visible.view.clone(),
        },
    )
    .await;
    let loading = publication(&mut events).await;
    tree(
        &commands,
        TreeCommand::Branch {
            view: visible.view,
            request: TreeRequestId(1),
            row: drivers,
            action: TreeBranchAction::Expand,
        },
    )
    .await;
    tree(&commands, TreeCommand::Commit(loading.view)).await;
    loop {
        let next = publication(&mut events).await;
        tree(&commands, TreeCommand::Commit(next.view.clone())).await;
        if next.view.rows.iter().any(|row| row.parent == Some(drivers)) {
            assert!(next.view.rows.iter().all(|row| row.error.is_none()));
            break;
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reproject_cannot_overtake_an_unacknowledged_provider_publication() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("drivers")).unwrap();
    std::fs::create_dir(directory.path().join("drivers").join("child")).unwrap();
    let (commands, mut events, engine, visible) = start(directory.path()).await;
    let row = visible.view.rows[0].id;
    tree(&commands, TreeCommand::Commit(visible.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Viewport {
            view: visible.view.clone(),
            rows: TreeViewport::new(&visible.view, vec![row]).unwrap(),
        },
    )
    .await;
    let loading = publication(&mut events).await;
    tree(
        &commands,
        TreeCommand::Reproject(TreeReprojectRequest {
            view: visible.view,
            request: TreeRequestId(1),
            selection_revision: visible.selection_revision,
            selection: visible.selection,
            options: TreeProjectionOptions::default(),
        }),
    )
    .await;
    assert!(matches!(
        event(&mut events).await,
        TreeEvent::ReprojectFailed {
            request: TreeRequestId(1),
            ..
        }
    ));
    tree(&commands, TreeCommand::Commit(loading.view)).await;
    let completed = publication(&mut events).await;
    tree(&commands, TreeCommand::Commit(completed.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Reproject(TreeReprojectRequest {
            view: completed.view.clone(),
            request: TreeRequestId(2),
            selection_revision: completed.selection_revision,
            selection: completed.selection,
            options: TreeProjectionOptions::default(),
        }),
    )
    .await;
    let TreeEvent::Reprojected {
        publication: reprojected,
        ..
    } = event(&mut events).await
    else {
        panic!("latest options request must succeed")
    };
    assert!(reprojected.view.stamp.revision.0 > completed.view.stamp.revision.0);
    tree(&commands, TreeCommand::Commit(reprojected.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Select(TreeSelectionRequest {
            view: reprojected.view,
            revision: SelectionRevision(1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action: TreeSelectionAction::Toggle(row),
        }),
    )
    .await;
    assert!(
        matches!(event(&mut events).await, TreeEvent::SelectionChanged { .. }),
        "the reproject ACK must not publish an older pending view"
    );
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}
