use super::*;
use super::{Command, Event};
use midnight_core::*;
use std::{
    collections::HashMap,
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

#[derive(Debug, Default)]
struct Mount {
    rows: HashMap<u64, Vec<ArchiveNode>>,
    lists: AtomicUsize,
    reads: AtomicUsize,
    changed: AtomicBool,
    blocked: AtomicBool,
    entered: AtomicBool,
}
impl ArchiveMount for Mount {
    fn id(&self) -> u64 {
        23
    }
    fn list(&self, id: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.lists.fetch_add(1, Ordering::Relaxed);
        self.entered.store(true, Ordering::Release);
        while self.blocked.load(Ordering::Acquire) && !cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(1));
        }
        if cancel.is_cancelled() {
            return Err(FsError::from_io(
                "test",
                Path::new(""),
                std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"),
            ));
        }
        Ok(self.rows.get(&id).cloned().unwrap_or_default())
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        panic!("archive size metadata must not extract content")
    }
    fn validate(&self) -> Result<(), FsError> {
        if self.changed.load(Ordering::Acquire) {
            Err(error("changed"))
        } else {
            Ok(())
        }
    }
}
fn error(message: &str) -> FsError {
    FsError::from_io("test", Path::new(""), std::io::Error::other(message))
}
fn node(id: u64, parent: u64, kind: EntryKind, size: Option<u64>) -> ArchiveNode {
    ArchiveNode {
        id: VirtualEntryId {
            mount: 23,
            entry: id,
        },
        parent,
        raw_name: Arc::from(b"duplicate".as_slice()),
        display_name: "duplicate".into(),
        kind,
        size,
        readable: kind == EntryKind::File,
        modified: None,
    }
}
fn fixture() -> Arc<Mount> {
    let mut mount = Mount::default();
    mount.rows.insert(
        2,
        vec![
            node(5, 2, EntryKind::File, Some(7)),
            node(6, 2, EntryKind::Directory, None),
        ],
    );
    mount
        .rows
        .insert(6, vec![node(7, 6, EntryKind::File, Some(11))]);
    mount
        .rows
        .insert(3, vec![node(8, 3, EntryKind::File, Some(100))]);
    Arc::new(mount)
}
fn root(
    snapshots: &Snapshots,
    mount: Arc<Mount>,
    navigation: u64,
    reverse: bool,
) -> TreeRootSnapshot {
    let mut entries = vec![
        Entry::from_archive(node(2, 1, EntryKind::Directory, None)),
        Entry::from_archive(node(3, 1, EntryKind::Directory, None)),
        Entry::from_archive(node(4, 1, EntryKind::File, Some(1))),
    ];
    if reverse {
        entries.reverse();
    }
    let listing = ArchiveListing {
        location: ArchiveLocation {
            mount: Some(mount),
            address: ArchiveAddress {
                provider: "test.archive".into(),
                format: Some(ArchiveFormat::Zip),
                container: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/test.zip"),
                source_version: vec![1],
                components: vec![],
            },
            directory: 1,
            trail: vec![(1, "test.zip".into())],
        },
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries: entries.into(),
    };
    let result = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(navigation),
        entries: listing.entries.clone(),
        index: listing.index.clone(),
    };
    let mut snapshots = snapshots.lock().unwrap();
    snapshots[0].archive = Some((result.navigation, result.entries.clone()));
    snapshots[0].archive_navigation = Some(result.navigation);
    snapshots[0].remember_archive_tree_root(PaneId::Left, result.navigation, &listing);
    result
}
fn setup() -> (
    Controller,
    Snapshots,
    tokio::sync::mpsc::Receiver<midnight_core::Event>,
) {
    let snapshots = Arc::new(Mutex::new(Default::default()));
    let (sender, events) = tokio::sync::mpsc::channel(64);
    let controller = Controller::new(
        snapshots.clone(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
    );
    (controller, snapshots, events)
}
async fn raw_next(events: &mut tokio::sync::mpsc::Receiver<midnight_core::Event>) -> Event {
    match tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap()
    {
        midnight_core::Event::ArchiveSizes(event) => *event,
        _ => panic!("unexpected event"),
    }
}
async fn next(
    controller: &Controller,
    events: &mut tokio::sync::mpsc::Receiver<midnight_core::Event>,
) -> Event {
    let event = raw_next(events).await;
    if let Event::Snapshot { pane, sizes, .. } = &event {
        controller
            .submit(Command::Acknowledge {
                pane: *pane,
                mount: sizes.mount,
                version: sizes.version,
            })
            .await;
    }
    event
}
async fn finished(
    controller: &Controller,
    events: &mut tokio::sync::mpsc::Receiver<midnight_core::Event>,
    scan: u64,
) -> Vec<Event> {
    let mut result = Vec::new();
    loop {
        let event = next(controller, events).await;
        let done = matches!(&event,Event::Finished{scan:id,..}if id.0==scan);
        result.push(event);
        if done {
            return result;
        }
    }
}
fn snapshot(events: &[Event]) -> Arc<ArchiveSizeSnapshot> {
    events
        .iter()
        .find_map(|event| match event {
            Event::Snapshot { sizes, .. } => Some(sizes.clone()),
            _ => None,
        })
        .unwrap()
}

#[tokio::test]
async fn selected_only_and_space_share_subdirectory_cache_without_content_reads() {
    let (controller, snapshots, mut events) = setup();
    let mount = fixture();
    let ready = root(&snapshots, mount.clone(), 1, false);
    controller
        .submit(Command::Compute {
            root: ready.clone(),
            scan: FolderSizeScanId(1),
            selection: Some(Arc::new(HashSet::from([EntryId(2)]))),
        })
        .await;
    let first = finished(&controller, &mut events, 1).await;
    let sizes = snapshot(&first);
    assert_eq!(
        sizes
            .get(VirtualEntryId {
                mount: 23,
                entry: 2
            })
            .unwrap()
            .bytes,
        18
    );
    assert_eq!(
        sizes
            .get(VirtualEntryId {
                mount: 23,
                entry: 6
            })
            .unwrap()
            .bytes,
        11
    );
    assert!(
        sizes
            .get(VirtualEntryId {
                mount: 23,
                entry: 3
            })
            .is_none()
    );
    assert_eq!(mount.lists.load(Ordering::Relaxed), 2);
    controller
        .submit(Command::Entry {
            root: ready,
            scan: FolderSizeScanId(2),
            entry: EntryId(2),
        })
        .await;
    let second = finished(&controller, &mut events, 2).await;
    assert!(snapshot(&second).version > sizes.version);
    assert_eq!(mount.lists.load(Ordering::Relaxed), 2);
    assert_eq!(mount.reads.load(Ordering::Relaxed), 0);
    controller.shutdown().await;
}
#[tokio::test]
async fn empty_and_files_only_selection_do_not_fall_back_to_all_directories() {
    let (controller, snapshots, mut events) = setup();
    let mount = fixture();
    let ready = root(&snapshots, mount.clone(), 1, false);
    for (scan, selected) in [(1, HashSet::new()), (2, HashSet::from([EntryId(4)]))] {
        controller
            .submit(Command::Compute {
                root: ready.clone(),
                scan: FolderSizeScanId(scan),
                selection: Some(Arc::new(selected)),
            })
            .await;
        assert!(snapshot(&finished(&controller, &mut events, scan).await).is_empty());
    }
    assert_eq!(mount.lists.load(Ordering::Relaxed), 0);
    controller.shutdown().await;
}
#[tokio::test]
async fn resort_reuses_identity_cache_and_observation_is_backend_summary() {
    let (controller, snapshots, mut events) = setup();
    let mount = fixture();
    let ready = root(&snapshots, mount.clone(), 1, false);
    controller
        .submit(Command::Compute {
            root: ready,
            scan: FolderSizeScanId(1),
            selection: None,
        })
        .await;
    let before = snapshot(&finished(&controller, &mut events, 1).await);
    let listed = mount.lists.load(Ordering::Relaxed);
    let sorted = root(&snapshots, mount.clone(), 2, true);
    controller
        .submit(Command::Observe {
            root: sorted,
            request: SelectionSizeId(1),
            revision: SelectionRevision(4),
            selected: Arc::new(HashSet::from([EntryId(2), EntryId(3)])),
        })
        .await;
    let Event::Snapshot { sizes, .. } = next(&controller, &mut events).await else {
        panic!()
    };
    assert!(sizes.version > before.version);
    let Event::Selected {
        navigation,
        revision,
        summary,
        ..
    } = next(&controller, &mut events).await
    else {
        panic!()
    };
    assert_eq!(navigation, NavigationId(2));
    assert_eq!(revision, SelectionRevision(4));
    assert_eq!(summary.complete, 2);
    assert_eq!(summary.complete_bytes, Some(118));
    assert_eq!(mount.lists.load(Ordering::Relaxed), listed);
    controller.shutdown().await;
}
#[tokio::test]
async fn reconstructed_stale_and_descendant_ids_are_rejected_without_fallback() {
    let (controller, snapshots, mut events) = setup();
    let mount = fixture();
    let ready = root(&snapshots, mount.clone(), 1, false);
    let mut forged = ready.clone();
    forged.entries = ready.entries.to_vec().into();
    controller
        .submit(Command::Compute {
            root: forged,
            scan: FolderSizeScanId(1),
            selection: None,
        })
        .await;
    assert!(matches!(
        next(&controller, &mut events).await,
        Event::Rejected { .. }
    ));
    controller
        .submit(Command::Entry {
            root: ready.clone(),
            scan: FolderSizeScanId(2),
            entry: EntryId(6),
        })
        .await;
    assert!(
        finished(&controller, &mut events, 2)
            .await
            .iter()
            .any(|e| matches!(e, Event::Rejected { .. }))
    );
    snapshots.lock().unwrap()[0].archive_navigation = Some(NavigationId(2));
    controller
        .submit(Command::Compute {
            root: ready,
            scan: FolderSizeScanId(3),
            selection: None,
        })
        .await;
    assert!(matches!(
        next(&controller, &mut events).await,
        Event::Rejected { .. }
    ));
    assert_eq!(mount.lists.load(Ordering::Relaxed), 0);
    controller.shutdown().await;
}
#[tokio::test]
async fn changed_source_never_reuses_completed_cache() {
    let (controller, snapshots, mut events) = setup();
    let mount = fixture();
    let ready = root(&snapshots, mount.clone(), 1, false);
    controller
        .submit(Command::Compute {
            root: ready.clone(),
            scan: FolderSizeScanId(1),
            selection: None,
        })
        .await;
    finished(&controller, &mut events, 1).await;
    mount.changed.store(true, Ordering::Release);
    controller
        .submit(Command::Entry {
            root: ready,
            scan: FolderSizeScanId(2),
            entry: EntryId(2),
        })
        .await;
    let result = finished(&controller, &mut events, 2).await;
    assert!(matches!(
        result.first(),
        Some(Event::Invalidated { mount: 23, .. })
    ));
    assert!(result.iter().all(|e| !matches!(e, Event::Snapshot { .. })));
    assert!(result.iter().any(|e| matches!(e, Event::Rejected { .. })));
    controller.shutdown().await;
}

#[tokio::test]
async fn bounded_queue_rejects_overflow_and_full_scan_supersedes_pending_requests() {
    let (controller, snapshots, mut events) = setup();
    let mount = fixture();
    mount.blocked.store(true, Ordering::Release);
    let ready = root(&snapshots, mount.clone(), 1, false);
    controller
        .submit(Command::Compute {
            root: ready.clone(),
            scan: FolderSizeScanId(1),
            selection: None,
        })
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !mount.entered.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    for scan in 2..=9 {
        controller
            .submit(Command::Entry {
                root: ready.clone(),
                scan: FolderSizeScanId(scan),
                entry: EntryId(2),
            })
            .await;
    }
    controller
        .submit(Command::Entry {
            root: ready.clone(),
            scan: FolderSizeScanId(10),
            entry: EntryId(2),
        })
        .await;
    assert!(matches!(
        next(&controller, &mut events).await,
        Event::Rejected {
            scan: Some(FolderSizeScanId(10)),
            ..
        }
    ));
    controller
        .submit(Command::Compute {
            root: ready,
            scan: FolderSizeScanId(11),
            selection: Some(Arc::new(HashSet::new())),
        })
        .await;
    let outcomes = finished(&controller, &mut events, 11).await;
    for scan in 2..=9 {
        assert!(
            outcomes.iter().any(
                |event| matches!(event,Event::Finished{scan:id,cancelled:true,..}if id.0==scan)
            )
        );
    }
    assert_eq!(mount.reads.load(Ordering::Relaxed), 0);
    controller.shutdown().await;
}

#[test]
fn selected_summary_overflow_is_unknown_and_equal_ids_do_not_reuse_other_mount_objects() {
    let snapshots = Arc::new(Mutex::new(Default::default()));
    let mount = fixture();
    let ready = root(&snapshots, mount.clone(), 1, false);
    let captured = snapshots.lock().unwrap()[0]
        .capture_tree_root(&ready)
        .unwrap();
    let observation = Observation {
        ready: Arc::downgrade(&captured),
        request: SelectionSizeId(1),
        revision: SelectionRevision(1),
        selected: Arc::new(HashSet::from([EntryId(2), EntryId(3)])),
    };
    let mut store = cache::Store::default();
    let authority: Arc<dyn ArchiveMount> = mount;
    for entry in [2, 3] {
        store.get(&authority, None).values.insert(
            VirtualEntryId { mount: 23, entry },
            FolderSize {
                bytes: u64::MAX,
                finished: true,
                ..FolderSize::default()
            },
        );
    }
    let summary = cache::summary(&observation, store.get(&authority, None));
    assert_eq!(summary.complete, 2);
    assert_eq!(summary.complete_bytes, None);
    let impostor: Arc<dyn ArchiveMount> = fixture();
    assert!(store.get(&impostor, None).values.is_empty());
}

#[tokio::test]
async fn observer_and_unacknowledged_publication_do_not_keep_a_departed_mount_alive() {
    let (controller, snapshots, mut events) = setup();
    let mount = fixture();
    let weak = Arc::downgrade(&mount);
    let ready = root(&snapshots, mount, 1, false);
    controller
        .submit(Command::Observe {
            root: ready,
            request: SelectionSizeId(1),
            revision: SelectionRevision(1),
            selected: Arc::new(HashSet::from([EntryId(2)])),
        })
        .await;
    assert!(matches!(
        raw_next(&mut events).await,
        Event::Snapshot { .. }
    ));
    assert!(matches!(
        raw_next(&mut events).await,
        Event::Selected { .. }
    ));
    snapshots.lock().unwrap()[0] = Default::default();
    tokio::time::timeout(Duration::from_secs(5), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    controller.shutdown().await;
}

#[tokio::test]
async fn selected_root_cannot_alias_a_captured_unselected_sibling() {
    let (controller, snapshots, mut events) = setup();
    let mut mount = Mount::default();
    mount
        .rows
        .insert(2, vec![node(3, 2, EntryKind::Directory, None)]);
    mount
        .rows
        .insert(3, vec![node(8, 3, EntryKind::File, Some(100))]);
    let mount = Arc::new(mount);
    let ready = root(&snapshots, mount.clone(), 1, false);
    controller
        .submit(Command::Compute {
            root: ready,
            scan: FolderSizeScanId(1),
            selection: Some(Arc::new(HashSet::from([EntryId(2)]))),
        })
        .await;
    let sizes = snapshot(&finished(&controller, &mut events, 1).await);
    assert_eq!(
        sizes
            .get(VirtualEntryId {
                mount: 23,
                entry: 2
            })
            .unwrap()
            .errors,
        1
    );
    assert!(
        sizes
            .get(VirtualEntryId {
                mount: 23,
                entry: 3
            })
            .is_none()
    );
    assert_eq!(mount.lists.load(Ordering::Relaxed), 1);
    controller.shutdown().await;
}

#[tokio::test]
async fn only_one_snapshot_is_in_flight_and_exact_ack_flushes_the_latest_cache() {
    let (controller, snapshots, mut events) = setup();
    let mount = fixture();
    let ready = root(&snapshots, mount, 1, false);
    controller
        .submit(Command::Compute {
            root: ready.clone(),
            scan: FolderSizeScanId(1),
            selection: None,
        })
        .await;
    let Event::Snapshot { sizes, .. } = raw_next(&mut events).await else {
        panic!()
    };
    assert!(matches!(
        raw_next(&mut events).await,
        Event::Finished { .. }
    ));
    for id in 1..=40 {
        controller
            .submit(Command::Observe {
                root: ready.clone(),
                request: SelectionSizeId(id),
                revision: SelectionRevision(id),
                selected: Arc::new(HashSet::from([EntryId(2)])),
            })
            .await;
    }
    controller
        .submit(Command::Acknowledge {
            pane: PaneId::Left,
            mount: 24,
            version: sizes.version,
        })
        .await;
    controller
        .submit(Command::Acknowledge {
            pane: PaneId::Left,
            mount: 23,
            version: sizes.version + 1,
        })
        .await;
    assert_eq!(
        controller.lanes[0].pending.lock().unwrap().in_flight,
        Some((23, sizes.version))
    );
    assert!(
        events.try_recv().is_err(),
        "blocked snapshots must not clone/enqueue another map"
    );
    controller
        .submit(Command::Acknowledge {
            pane: PaneId::Left,
            mount: 23,
            version: sizes.version,
        })
        .await;
    let Event::Snapshot { sizes: latest, .. } = raw_next(&mut events).await else {
        panic!()
    };
    assert_eq!(latest.version, sizes.version + 1);
    assert_eq!(
        latest
            .get(VirtualEntryId {
                mount: 23,
                entry: 2
            })
            .unwrap()
            .bytes,
        18
    );
    controller.shutdown().await;
}

#[tokio::test]
async fn newer_same_mount_observer_keeps_final_results_from_an_older_running_capture() {
    let (controller, snapshots, mut events) = setup();
    let mount = fixture();
    mount.blocked.store(true, Ordering::Release);
    let ready = root(&snapshots, mount.clone(), 1, false);
    controller
        .submit(Command::Compute {
            root: ready,
            scan: FolderSizeScanId(1),
            selection: Some(Arc::new(HashSet::from([EntryId(2)]))),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !mount.entered.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let sorted = root(&snapshots, mount.clone(), 2, true);
    controller
        .submit(Command::Observe {
            root: sorted,
            request: SelectionSizeId(1),
            revision: SelectionRevision(1),
            selected: Arc::new(HashSet::from([EntryId(2)])),
        })
        .await;
    tokio::time::sleep(Duration::from_millis(260)).await;
    mount.blocked.store(false, Ordering::Release);
    let (mut complete, mut done) = (false, false);
    while !complete || !done {
        match next(&controller, &mut events).await {
            Event::Snapshot { sizes, .. } => {
                complete |= sizes
                    .get(VirtualEntryId {
                        mount: 23,
                        entry: 2,
                    })
                    .is_some_and(|size| size.finished && size.bytes == 18)
            }
            Event::Finished {
                scan: FolderSizeScanId(1),
                cancelled: false,
                ..
            } => done = true,
            _ => {}
        }
    }
    assert_eq!(mount.reads.load(Ordering::Relaxed), 0);
    controller.shutdown().await;
}
#[tokio::test]
async fn sorting_does_not_cancel_active_capture_but_explicit_cancel_does() {
    let (controller, snapshots, mut events) = setup();
    let mount = fixture();
    mount.blocked.store(true, Ordering::Release);
    let ready = root(&snapshots, mount.clone(), 1, false);
    controller
        .submit(Command::Compute {
            root: ready,
            scan: FolderSizeScanId(1),
            selection: None,
        })
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !mount.entered.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    root(&snapshots, mount.clone(), 2, true);
    assert!(events.try_recv().is_err());
    controller
        .submit(Command::Cancel {
            pane: PaneId::Left,
            scan: FolderSizeScanId(1),
        })
        .await;
    let result = finished(&controller, &mut events, 1).await;
    assert!(result.iter().any(|e| matches!(
        e,
        Event::Finished {
            cancelled: true,
            ..
        }
    )));
    let partial = snapshot(&result);
    let cancelled = partial
        .get(VirtualEntryId {
            mount: 23,
            entry: 2,
        })
        .expect("the started root retains its partial result");
    assert!(
        !cancelled.finished,
        "cancellation cannot complete an unread root"
    );
    assert_eq!(cancelled.errors, 0, "cancellation is not a read error");
    assert_eq!(mount.reads.load(Ordering::Relaxed), 0);
    controller.shutdown().await;
}
