//! A stale target/source must retire captured provider ownership on rejection.
use super::*;

#[tokio::test]
async fn stale_internal_drop_releases_capture_without_waiting_for_another_gesture() {
    rejected_capture(false).await;
}

#[tokio::test]
async fn failed_preparation_releases_capture_without_waiting_for_another_gesture() {
    rejected_capture(true).await;
}

async fn rejected_capture(failed_preparation: bool) {
    let fixture = Harness::new();
    let dropped = Arc::new(Mutex::new(None));
    let source = Arc::new(CapturedSource {
        files: fixture.request.files.clone(),
        dropped: dropped.clone(),
    });
    let weak = Arc::downgrade(&source);
    let (sender, mut events) = mpsc::channel(128);
    let mut tasks = JoinSet::new();
    let mut controller = Controller::new(
        None,
        None,
        fixture.sink.snapshots.clone(),
        fixture.sink.capacity.clone(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    controller
        .attach(
            fixture.sink.host.clone(),
            fixture.sink.targets.clone(),
            &mut tasks,
        )
        .await;
    if failed_preparation {
        fixture.sink.snapshots.lock().unwrap()[1].pending = Some(NavigationId(6));
    }
    controller
        .prepare_captured(DragId(1), PaneId::Right, NavigationId(5), source.clone())
        .await;
    drop(source);
    if !failed_preparation {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if matches!(
                    events.recv().await,
                    Some(Event::DragDrop(DragDropEvent::Prepared {
                        id: DragId(1),
                        ..
                    }))
                ) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        fixture.sink.snapshots.lock().unwrap()[1].pending = Some(NavigationId(6));
        controller
            .internal_request(InternalDropRequest {
                id: DragId(1),
                destination: InternalDropDestination::PaneRoot {
                    pane: PaneId::Right,
                    navigation: NavigationId(5),
                },
                intent: InternalDropIntent::Copy,
                verification: TransferVerification::FullSha256,
            })
            .await;
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Event::DragDrop(DragDropEvent::Failed {
                id: Some(DragId(1)),
                ..
            })) = events.recv().await
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    let retired = tokio::time::timeout(Duration::from_millis(500), async {
        while weak.upgrade().is_some() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(
        retired.is_ok(),
        "rejected drag retained provider until another gesture"
    );
    assert_ne!(
        dropped.lock().unwrap().unwrap(),
        std::thread::current().id()
    );
    assert_eq!(fixture.sink.capacity.capacity().available_permits(), 32);
    assert!(controller.pending.lock().unwrap().is_empty());
    controller.shutdown();
    while tasks.join_next().await.is_some() {}
}
