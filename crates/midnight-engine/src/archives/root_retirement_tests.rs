use super::*;
use std::sync::{Mutex, Weak};
use tokio::sync::oneshot;

struct RetiringArchive {
    snapshots: Weak<crate::retirement::WorkerOwned<Mutex<[crate::SnapshotState; 2]>>>,
    report: Option<oneshot::Sender<(std::thread::ThreadId, bool)>>,
}

impl std::fmt::Debug for RetiringArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RetiringArchive")
    }
}

impl Drop for RetiringArchive {
    fn drop(&mut self) {
        if let Some(report) = self.report.take() {
            let unlocked = self
                .snapshots
                .upgrade()
                .is_none_or(|states| states.try_lock().is_ok());
            let _ = report.send((std::thread::current().id(), unlocked));
        }
    }
}

impl ArchiveMount for RetiringArchive {
    fn id(&self) -> u64 {
        31
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        Ok(vec![])
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("navigation must not extract members")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}

fn location(mount: RetiringArchive) -> ArchiveLocation {
    ArchiveLocation {
        mount: Some(Arc::new(mount)),
        address: ArchiveAddress {
            format: Some(ArchiveFormat::Zip),
            provider: "test.archive".into(),
            container: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/retirement.zip"),
            source_version: vec![1],
            components: vec![],
        },
        directory: 1,
        trail: vec![(1, "retirement.zip".into())],
    }
}

#[tokio::test]
async fn replacing_archive_root_retires_mount_on_blocking_worker_outside_snapshot_lock() {
    let caller = std::thread::current().id();
    let snapshots: Snapshots = crate::test_snapshots(Default::default());
    let retirement = remember_old_root(&snapshots);
    let (sender, mut events) = tokio::sync::mpsc::channel(1);
    let catalog = OnceLock::new();
    catalog.set(Catalog::default()).unwrap();
    let mut controller = Controller {
        capture: None,
        locale: None,
        catalog: Arc::new(catalog),
        budget: Arc::new(Semaphore::new(1)),
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
        ArchiveAction::List(location(RetiringArchive {
            snapshots: Weak::new(),
            report: None,
        })),
        SortSpec::default(),
        &mut tasks,
    );
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .unwrap();
    let Some(Event::ArchiveListed {
        pane,
        navigation,
        result: Ok(listing),
    }) = event
    else {
        panic!("successful archive listing expected")
    };
    assert!(
        snapshots.lock().unwrap()[0]
            .capture_tree_root(&midnight_core::TreeRootSnapshot {
                pane,
                navigation,
                entries: listing.entries,
                index: listing.index,
            })
            .is_ok()
    );
    tasks.join_next().await.unwrap().unwrap();
    let (thread, unlocked) = retirement.await.unwrap();
    assert!(
        unlocked,
        "provider destruction must not run under the snapshot mutex"
    );
    assert_ne!(
        thread, caller,
        "provider destruction must not run on the async executor"
    );
}

fn remember_old_root(snapshots: &Snapshots) -> oneshot::Receiver<(std::thread::ThreadId, bool)> {
    let (report, retirement) = oneshot::channel();
    let listing = ArchiveListing {
        location: location(RetiringArchive {
            snapshots: Arc::downgrade(snapshots),
            report: Some(report),
        }),
        entries: Arc::from([]),
        index: Arc::default(),
    };
    {
        let mut states = snapshots.lock().unwrap();
        drop(states[0].remember_archive_tree_root(PaneId::Left, NavigationId(1), &listing));
        states[0].archive = Some((NavigationId(1), listing.entries.clone()));
    }
    drop(listing);
    retirement
}

#[tokio::test]
async fn shutdown_drains_cached_roots_off_executor_and_outside_state_lock() {
    let caller = std::thread::current().id();
    let snapshots: Snapshots = crate::test_snapshots(Default::default());
    let retirement = remember_old_root(&snapshots);
    crate::tree_roots::shutdown(snapshots.clone()).await;
    let (thread, unlocked) = retirement.await.unwrap();
    assert!(unlocked);
    assert_ne!(thread, caller);
    assert!(
        snapshots
            .lock()
            .unwrap()
            .iter()
            .all(|state| state.tree_root.is_none())
    );
}

#[tokio::test]
async fn native_publication_retires_previous_archive_outside_state_lock() {
    let snapshots: Snapshots = crate::test_snapshots(Default::default());
    let retirement = remember_old_root(&snapshots);
    let (sender, mut events) = tokio::sync::mpsc::channel(1);
    tokio::task::spawn_blocking(move || {
        crate::publish_listing(
            &Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &snapshots,
            None,
            PaneId::Left,
            SortSpec::default(),
            &Cancellation::default(),
            crate::CommittedSnapshot {
                navigation: NavigationId(2),
                path: Path::new(env!("CARGO_MANIFEST_DIR")).to_owned(),
                entries: Arc::from([]),
                sizes: Arc::default(),
            },
        );
    })
    .await
    .unwrap();
    assert!(matches!(
        events.try_recv(),
        Ok(Event::ListingFinished { .. })
    ));
    let (thread, unlocked) = retirement.await.unwrap();
    assert!(unlocked);
    assert_ne!(thread, std::thread::current().id());
}

#[test]
fn cancelled_or_superseded_archive_completion_never_refills_root_cache() {
    for cancelled in [false, true] {
        let snapshots: Snapshots = crate::test_snapshots(Default::default());
        snapshots.lock().unwrap()[0].archive_navigation =
            Some(NavigationId(if cancelled { 2 } else { 3 }));
        let cancel = Cancellation::default();
        if cancelled {
            cancel.cancel();
        }
        let listing = ArchiveListing {
            location: location(RetiringArchive {
                snapshots: Weak::new(),
                report: None,
            }),
            entries: Arc::from([]),
            index: Arc::default(),
        };
        remember_listing(&snapshots, PaneId::Left, NavigationId(2), &listing, &cancel);
        let states = snapshots.lock().unwrap();
        assert!(states[0].tree_root.is_none());
        assert!(states[0].archive.is_none());
    }
}

#[test]
fn last_snapshot_owner_retires_cached_archive_without_a_runtime() {
    let caller = std::thread::current().id();
    let snapshots = crate::test_snapshots(Default::default());
    let mut report = remember_old_root(&snapshots);
    let last_owner = snapshots.clone();
    drop(snapshots);
    drop(last_owner);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let (thread, unlocked) = loop {
        match report.try_recv() {
            Ok(value) => break value,
            Err(oneshot::error::TryRecvError::Empty) if std::time::Instant::now() < deadline => {
                std::thread::yield_now();
            }
            error => panic!("archive retirement did not finish: {error:?}"),
        }
    };
    assert_ne!(thread, caller);
    assert!(unlocked);
}
