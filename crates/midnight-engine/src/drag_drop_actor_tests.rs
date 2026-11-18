//! Without a native session the actor has nothing to pump, so it sleeps until a
//! controller change wakes it.
use super::*;
use crate::{CommittedSnapshot, SnapshotState};

async fn next(events: &mut mpsc::Receiver<Event>, expected: impl Fn(&DragDropEvent) -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(Event::DragDrop(event)) if expected(&event) => return,
                Some(_) => {}
                None => panic!("drag event stream ended"),
            }
        }
    })
    .await
    .expect("the drag event did not arrive");
}

#[tokio::test]
async fn without_a_native_session_the_actor_sleeps_until_a_controller_change() {
    let file = std::env::current_dir().unwrap().join("dragged 文档.txt");
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), file.clone(), EntryKind::File)].into();
    let snapshots = crate::test_snapshots([
        SnapshotState {
            committed: Some(CommittedSnapshot {
                navigation: NavigationId(5),
                path: file.parent().unwrap().to_owned(),
                entries: entries.clone(),
                sizes: Arc::default(),
            }),
            ..Default::default()
        },
        SnapshotState::default(),
    ]);
    let (sender, mut events) = mpsc::channel(16);
    let mut tasks = JoinSet::new();
    let mut controller = Controller::new(
        crate::engine_lifetime::DragRetirement::testing(),
        None,
        None,
        snapshots,
        copy_operations::Admission::default(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let host = |generation| DragHost {
        generation,
        native: NativeDragWindow::Unsupported,
        lifetime: Cancellation::default(),
    };
    let targets = SharedDropTargets::default();
    controller
        .attach(host(1), targets.clone(), &mut tasks)
        .await;
    next(&mut events, |event| {
        matches!(
            event,
            DragDropEvent::Ready {
                host_generation: 1,
                supported: false
            }
        )
    })
    .await;
    let wakeups = |controller: &Controller| controller.state.lock().unwrap().actor_wakeups;
    let asleep = wakeups(&controller);
    // An actor waking on a timer would wake several times in this window.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(wakeups(&controller), asleep, "the idle actor woke");

    // A queued start wakes it.
    controller
        .prepare(
            DragId(1),
            PaneId::Left,
            NavigationId(5),
            SelectedFiles {
                selection: Arc::default(),
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                fallback: Some(file),
            },
        )
        .await;
    next(&mut events, |event| {
        matches!(event, DragDropEvent::Prepared { id: DragId(1), .. })
    })
    .await;
    controller.start(DragId(1)).await;
    next(&mut events, |event| {
        matches!(
            event,
            DragDropEvent::Finished {
                id: DragId(1),
                outcome: DragFinish::Unavailable
            }
        )
    })
    .await;
    // So does a new host.
    controller.attach(host(2), targets, &mut tasks).await;
    next(&mut events, |event| {
        matches!(
            event,
            DragDropEvent::Ready {
                host_generation: 2,
                supported: false
            }
        )
    })
    .await;
    // And shutdown.
    controller.shutdown();
    tokio::time::timeout(Duration::from_secs(5), async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .expect("shutdown did not stop the actors");
}
