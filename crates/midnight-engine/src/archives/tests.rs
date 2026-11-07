use super::*;
use std::sync::Mutex;

async fn failed_navigation(action: ArchiveAction) -> crate::SnapshotState {
    let snapshots: Snapshots = Arc::new(Mutex::new(Default::default()));
    snapshots.lock().unwrap()[0].committed = Some(crate::CommittedSnapshot {
        navigation: NavigationId(1),
        path: std::env::temp_dir(),
        entries: Arc::from([]),
        sizes: Arc::default(),
    });
    let (sender, mut events) = tokio::sync::mpsc::channel(4);
    let catalog = OnceLock::new();
    catalog.set(Catalog::default()).unwrap();
    let mut controller = Controller {
        catalog: Arc::new(catalog),
        budget: Arc::new(Semaphore::new(2)),
        cancellations: std::array::from_fn(|_| Cancellation::default()),
        outbox: Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        snapshots: snapshots.clone(),
        ready: Arc::new(Notify::new()),
    };
    let mut tasks = JoinSet::new();
    controller.navigate(
        PaneId::Left,
        NavigationId(2),
        action,
        SortSpec::default(),
        &mut tasks,
    );
    assert!(matches!(
        events.recv().await,
        Some(Event::ArchiveListed { result: Err(_), .. })
    ));
    tasks.join_next().await.unwrap().unwrap();
    std::mem::take(&mut snapshots.lock().unwrap()[0])
}

#[tokio::test]
async fn failed_archive_open_restores_native_snapshot_capabilities() {
    let state = failed_navigation(ArchiveAction::Open {
        path: std::env::temp_dir().join("missing.zip"),
    })
    .await;
    assert!(state.archive_navigation.is_none());
    assert!(state.archive.is_none());
    assert_eq!(state.committed.unwrap().navigation, NavigationId(1));
}

#[tokio::test]
async fn unavailable_saved_archive_keeps_native_capabilities_disabled() {
    let state = failed_navigation(ArchiveAction::List(ArchiveLocation::unavailable(
        ArchiveAddress {
            provider: "missing.provider".into(),
            container: std::env::temp_dir().join("missing.zip"),
            source_version: vec![1],
            components: vec![],
        },
    )))
    .await;
    assert_eq!(state.archive_navigation, Some(NavigationId(2)));
}
