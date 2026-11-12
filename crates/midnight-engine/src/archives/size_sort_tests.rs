use super::*;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

#[derive(Debug, Default)]
struct Source {
    rows: HashMap<u64, Vec<ArchiveNode>>,
    changed: AtomicBool,
    change_on_list: AtomicBool,
    reads: AtomicUsize,
    retired: Option<tokio::sync::oneshot::Sender<std::thread::ThreadId>>,
}
impl Drop for Source {
    fn drop(&mut self) {
        if let Some(retired) = self.retired.take() {
            let _ = retired.send(std::thread::current().id());
        }
    }
}
impl ArchiveMount for Source {
    fn id(&self) -> u64 {
        73
    }
    fn validate(&self) -> Result<(), FsError> {
        if self.changed.load(Ordering::Acquire) {
            Err(error("source changed"))
        } else {
            Ok(())
        }
    }
    fn list(&self, id: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        if self.change_on_list.load(Ordering::Acquire) {
            self.changed.store(true, Ordering::Release);
        }
        Ok(self.rows.get(&id).cloned().unwrap_or_default())
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        panic!("manual archive size sorting must not extract content")
    }
}
fn node(id: u64, parent: u64, kind: EntryKind, size: Option<u64>) -> ArchiveNode {
    ArchiveNode {
        id: VirtualEntryId {
            mount: 73,
            entry: id,
        },
        parent,
        raw_name: Arc::from(b"same".as_slice()),
        display_name: "same".into(),
        kind,
        size,
        readable: kind == EntryKind::File,
        modified: None,
    }
}
fn source() -> Arc<Source> {
    Arc::new(Source {
        rows: HashMap::from([
            (
                1,
                vec![
                    node(2, 1, EntryKind::Directory, None),
                    node(3, 1, EntryKind::Directory, None),
                    node(4, 1, EntryKind::Directory, None),
                    node(5, 1, EntryKind::DirectoryLink, None),
                    node(6, 1, EntryKind::Directory, None),
                    node(7, 1, EntryKind::File, Some(8)),
                    node(8, 1, EntryKind::File, Some(2)),
                ],
            ),
            (2, vec![node(20, 2, EntryKind::File, Some(100))]),
            (3, vec![node(30, 3, EntryKind::File, Some(7))]),
            (4, vec![node(40, 4, EntryKind::File, None)]),
        ]),
        changed: AtomicBool::new(false),
        change_on_list: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
        retired: None,
    })
}
fn location(source: Arc<Source>) -> ArchiveLocation {
    ArchiveLocation {
        mount: Some(source),
        address: ArchiveAddress {
            provider: "test.sizes".into(),
            format: Some(ArchiveFormat::Zip),
            container: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/owned.zip"),
            source_version: vec![1],
            components: vec![],
        },
        directory: 1,
        trail: vec![(1, "owned.zip".into())],
    }
}
struct Harness {
    snapshots: Snapshots,
    navigation: Controller,
    sizes: crate::archive_sizes::Controller,
    events: tokio::sync::mpsc::Receiver<Event>,
    tasks: JoinSet<()>,
    generation: u64,
}
impl Harness {
    fn new() -> Self {
        let snapshots: Snapshots = crate::test_snapshots(Default::default());
        let (sender, events) = tokio::sync::mpsc::channel(32);
        let outbox = Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        };
        let size_domain = crate::retirement::RetirementDomain::try_start(
            crate::archive_sizes::RETIREMENT_CAPACITY,
            0,
        )
        .unwrap();
        Self {
            navigation: Controller::testing(
                outbox.clone(),
                snapshots.clone(),
                Arc::new(Semaphore::new(2)),
            ),
            sizes: crate::archive_sizes::Controller::new(
                crate::archive_sizes::Retirement {
                    panes: std::array::from_fn(|_| size_domain.try_reserve().unwrap()),
                },
                crate::ReportArena::testing(),
                snapshots.clone(),
                outbox,
            ),
            snapshots,
            events,
            tasks: JoinSet::new(),
            generation: 1,
        }
    }
    async fn event(&mut self) -> Event {
        tokio::time::timeout(Duration::from_secs(5), self.events.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn compute(&mut self, source: Arc<Source>) -> Arc<ArchiveSizeSnapshot> {
        let entries: Arc<[Entry]> = source.rows[&1]
            .iter()
            .cloned()
            .map(Entry::from_archive)
            .collect();
        let listing = ArchiveListing {
            location: location(source),
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        };
        let root = TreeRootSnapshot {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            entries: listing.entries.clone(),
            index: listing.index.clone(),
        };
        {
            let mut states = self.snapshots.lock().unwrap();
            let state = &mut states[0];
            state.archive = Some((root.navigation, root.entries.clone()));
            state.archive_navigation = Some(root.navigation);
            state.remember_archive_tree_root(root.pane, root.navigation, &listing);
        }
        self.sizes
            .submit(ArchiveSizeCommand::Compute {
                root,
                scan: FolderSizeScanId(1),
                selection: None,
            })
            .await;
        let mut latest = None;
        loop {
            let Event::ArchiveSizes(event) = self.event().await else {
                panic!("unexpected event")
            };
            match *event {
                ArchiveSizeEvent::Snapshot { sizes, pane, .. } => {
                    self.sizes
                        .submit(ArchiveSizeCommand::Acknowledge {
                            pane,
                            mount: sizes.mount,
                            version: sizes.version,
                        })
                        .await;
                    latest = Some(sizes);
                }
                ArchiveSizeEvent::Finished { cancelled, .. } => {
                    assert!(!cancelled);
                    return latest.unwrap();
                }
                _ => {}
            }
        }
    }
    async fn sort(
        &mut self,
        source: Arc<Source>,
        descending: bool,
    ) -> Result<ArchiveListing, FsError> {
        self.generation += 1;
        self.navigation
            .navigate(
                PaneId::Left,
                NavigationId(self.generation),
                ArchiveAction::List(location(source)),
                SortSpec {
                    column: SortColumn::Size,
                    descending,
                },
                &mut self.tasks,
            )
            .await;
        let Event::ArchiveListed { result, .. } = self.event().await else {
            panic!("unexpected event")
        };
        self.tasks.join_next().await.unwrap().unwrap();
        result
    }
}

#[tokio::test]
async fn explicit_size_sort_uses_published_complete_totals_and_preserves_member_metadata() {
    let source = source();
    let mut harness = Harness::new();
    let snapshot = harness.compute(source.clone()).await;
    assert_eq!(
        snapshot
            .get(VirtualEntryId {
                mount: 73,
                entry: 2
            })
            .unwrap()
            .bytes,
        100
    );
    assert_eq!(
        harness.snapshots.lock().unwrap()[0]
            .archive
            .as_ref()
            .unwrap()
            .1[0]
            .id,
        EntryId(2),
        "size publication must not resort the captured listing"
    );
    assert!(
        harness.events.try_recv().is_err(),
        "no automatic navigation event"
    );
    for (descending, expected) in [
        (false, vec![6, 3, 2, 4, 5, 8, 7]),
        (true, vec![2, 3, 6, 4, 5, 7, 8]),
    ] {
        let listing = harness.sort(source.clone(), descending).await.unwrap();
        assert_eq!(
            listing
                .entries
                .iter()
                .map(|entry| entry.id.0)
                .collect::<Vec<_>>(),
            expected
        );
        for entry in listing.entries.iter() {
            let original = source.rows[&1]
                .iter()
                .find(|node| node.id.entry == entry.id.0)
                .unwrap();
            assert_eq!(entry.metadata.size, original.size);
            assert_eq!(entry.virtual_node.as_ref().unwrap().size, original.size);
        }
    }
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    harness.sizes.shutdown().await;
}

#[tokio::test]
async fn replacement_mount_with_identical_numeric_id_does_not_inherit_totals() {
    let mut harness = Harness::new();
    let snapshot = harness.compute(source()).await;
    let replacement = source();
    let listing = harness.sort(replacement.clone(), false).await.unwrap();
    assert_eq!(
        listing
            .entries
            .iter()
            .map(|entry| entry.id.0)
            .collect::<Vec<_>>(),
        vec![2, 3, 4, 5, 6, 8, 7]
    );
    assert!(!snapshot.is_empty());
    assert_eq!(replacement.reads.load(Ordering::Relaxed), 0);
    harness.sizes.shutdown().await;
}

#[tokio::test]
async fn source_changes_before_or_during_listing_reject_measured_sort() {
    for during_listing in [false, true] {
        let mut harness = Harness::new();
        let source = source();
        let snapshot = harness.compute(source.clone()).await;
        if during_listing {
            source.change_on_list.store(true, Ordering::Release);
        } else {
            source.changed.store(true, Ordering::Release);
        }
        assert!(harness.sort(source.clone(), false).await.is_err());
        assert!(!snapshot.is_empty());
        assert_eq!(source.reads.load(Ordering::Relaxed), 0);
        harness.sizes.shutdown().await;
    }
}

struct RetirementGate {
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    released: std::sync::mpsc::Receiver<()>,
}
impl Drop for RetirementGate {
    fn drop(&mut self) {
        let _ = self.entered.take().unwrap().send(());
        // Dropping the sender also releases the worker if the test panics.
        let _ = self.released.recv();
    }
}

#[tokio::test]
async fn published_sort_lookup_does_not_retain_retired_map_or_mount() {
    let mut harness = Harness::new();
    let (retired, completed) = tokio::sync::oneshot::channel();
    let mut source = source();
    Arc::get_mut(&mut source).unwrap().retired = Some(retired);
    let weak_source = Arc::downgrade(&source);
    let snapshot = harness.compute(source.clone()).await;
    let weak_snapshot = Arc::downgrade(&snapshot);
    drop(snapshot);
    assert!(weak_snapshot.upgrade().is_none());
    // Task completion must not imply destruction of its queued native owner.
    let (release_retirement, released) = std::sync::mpsc::channel();
    let (entered, retirement_entered) = tokio::sync::oneshot::channel();
    let gate_finished = harness
        .navigation
        .retirement
        .domain
        .try_reserve()
        .unwrap()
        .retire(RetirementGate {
            entered: Some(entered),
            released,
        });
    tokio::time::timeout(Duration::from_secs(5), retirement_entered)
        .await
        .expect("retirement worker must enter the fixture gate")
        .unwrap();
    let listing = harness.sort(source.clone(), false).await.unwrap();
    assert_eq!(
        listing.entries[0].id,
        EntryId(2),
        "retired totals are unavailable"
    );
    drop(listing);
    drop(source);
    harness.sizes.shutdown().await;
    drop(harness.navigation);
    // The joined navigation task can still own snapshots in its retirement queue.
    // Drain the cached roots through the real shutdown contract without assuming
    // that this test already owns the last snapshot reference.
    crate::tree_roots::shutdown(harness.snapshots.clone()).await;
    drop(harness.snapshots);
    assert!(
        weak_source.upgrade().is_some(),
        "the guarded mount must still await the held retirement worker"
    );
    drop(release_retirement);
    tokio::time::timeout(Duration::from_secs(5), gate_finished)
        .await
        .expect("retirement worker must leave the fixture gate")
        .unwrap();
    // The root drain only enqueues the guarded mount release; wait for the actual
    // source destructor rather than assuming its worker has already finished.
    let retired_on = tokio::time::timeout(Duration::from_secs(5), completed)
        .await
        .expect("archive mount must retire after its final owner is released")
        .unwrap();
    assert_ne!(retired_on, std::thread::current().id());
    assert!(weak_source.upgrade().is_none());
}
