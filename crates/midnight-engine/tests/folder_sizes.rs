use midnight_core::{
    Cancellation, Command, DirectoryService, Entry, EntryId, EntryKind, EntryMetadata, Event,
    FolderSizeScanId, FsError, NavigationId, PaneId, SortSpec, SpaceInfo, StorageService, Volume,
    VolumeService,
};
use midnight_engine::Engine;
use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, mpsc};

async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("Engine must remain responsive")
        .expect("Engine event stream closed unexpectedly")
}

fn navigate(pane: PaneId, navigation: u64, path: &Path) -> Command {
    Command::Navigate {
        pane,
        navigation: NavigationId(navigation),
        path: path.into(),
        sort: SortSpec::default(),
    }
}

fn scan(pane: PaneId, navigation: u64, generation: u64) -> Command {
    Command::ComputeFolderSizes {
        pane,
        navigation: NavigationId(navigation),
        scan: FolderSizeScanId(generation),
        selection: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn individual_directory_scans_keep_other_results_and_ignore_file_rows() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path();
    for (folder, bytes) in [("first", 7usize), ("second", 11)] {
        fs::create_dir(base.join(folder)).unwrap();
        fs::write(base.join(folder).join("文档.txt"), vec![1u8; bytes]).unwrap();
    }
    fs::write(base.join("ordinary.txt"), [1u8; 99]).unwrap();
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(midnight_platform::LocalFileSystem::new(None)),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, base))
        .await
        .unwrap();
    let entries = listing(&mut events, PaneId::Left, 1).await;
    for (index, entry) in entries.iter().enumerate() {
        commands
            .send(Command::ComputeEntrySize {
                pane: PaneId::Left,
                navigation: NavigationId(1),
                scan: FolderSizeScanId(index as u64 + 1),
                entry: entry.id,
            })
            .await
            .unwrap();
    }
    let mut finished = HashMap::new();
    let mut sizes = HashMap::new();
    while finished.len() < entries.len() {
        match next(&mut events).await {
            Event::FolderSizeUpdated {
                scan, entry, size, ..
            } if size.finished => {
                assert_ne!(
                    entries.iter().find(|row| row.id == entry).unwrap().kind,
                    EntryKind::File
                );
                sizes.insert(scan.0, size.bytes);
            }
            Event::FolderSizesFinished {
                scan, cancelled, ..
            } => {
                finished.insert(scan.0, cancelled);
            }
            _ => {}
        }
    }
    assert_eq!(sizes.values().copied().sum::<u64>(), 18);
    assert_eq!(sizes.len(), 2);
    assert_eq!(finished.values().filter(|cancelled| **cancelled).count(), 1);
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

async fn listing(
    events: &mut mpsc::Receiver<Event>,
    pane: PaneId,
    generation: u64,
) -> Arc<[Entry]> {
    loop {
        match next(events).await {
            Event::ListingFinished {
                pane: actual,
                navigation: NavigationId(id),
                entries,
                ..
            } if actual == pane && id == generation => return entries,
            Event::ListingFailed { error, .. } => panic!("Test listing failed: {error}"),
            _ => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_unicode_tree_computes_only_visible_directory_rows_with_stable_identity() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path();
    fs::create_dir_all(base.join("文档/👩🏽‍💻")).unwrap();
    fs::create_dir(base.join("prázdná")).unwrap();
    fs::write(base.join("文档/Žluťoučký.txt"), [1u8; 7]).unwrap();
    fs::write(base.join("文档/👩🏽‍💻/مرحبا.txt"), [2u8; 11]).unwrap();
    fs::write(base.join("unrelated.txt"), [3u8; 99]).unwrap();
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(midnight_platform::LocalFileSystem::new(Some(base.into()))),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, base))
        .await
        .unwrap();
    let entries = listing(&mut events, PaneId::Left, 1).await;
    let identities = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Directory)
        .map(|entry| (entry.id, entry.path.clone()))
        .collect::<HashMap<_, _>>();
    commands.send(scan(PaneId::Left, 1, 7)).await.unwrap();
    let mut sizes = HashMap::new();
    loop {
        match next(&mut events).await {
            Event::FolderSizeUpdated {
                pane,
                navigation,
                scan,
                entry,
                size,
            } => {
                assert_eq!(pane, PaneId::Left);
                assert_eq!(navigation, NavigationId(1));
                assert_eq!(scan, FolderSizeScanId(7));
                assert!(identities.contains_key(&entry));
                if size.finished {
                    assert_eq!(size.errors, 0);
                    sizes.insert(identities[&entry].clone(), size);
                }
            }
            Event::FolderSizesFinished {
                pane,
                navigation,
                scan,
                cancelled,
            } => {
                assert_eq!(
                    (pane, navigation, scan),
                    (PaneId::Left, NavigationId(1), FolderSizeScanId(7))
                );
                assert!(!cancelled);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(sizes.len(), 2);
    assert_eq!(sizes[&base.join("文档")].bytes, 18);
    assert_eq!(sizes[&base.join("文档")].files, 2);
    assert_eq!(sizes[&base.join("prázdná")].bytes, 0);
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_navigation_preserves_the_committed_snapshot_for_size_scans() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path();
    fs::create_dir(base.join("folder")).unwrap();
    fs::write(base.join("folder/file"), [1u8; 9]).unwrap();
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(midnight_platform::LocalFileSystem::new(Some(base.into()))),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, base))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 1).await;
    commands
        .send(navigate(PaneId::Left, 2, &base.join("missing")))
        .await
        .unwrap();
    loop {
        if let Event::ListingFailed {
            navigation: NavigationId(2),
            ..
        } = next(&mut events).await
        {
            break;
        }
    }
    commands.send(scan(PaneId::Left, 1, 1)).await.unwrap();
    let mut final_size = None;
    loop {
        match next(&mut events).await {
            Event::FolderSizeUpdated { size, .. } if size.finished => final_size = Some(size.bytes),
            Event::FolderSizesFinished { cancelled, .. } => {
                assert!(
                    !cancelled,
                    "Rolled-back pane must remain usable for folder sizing"
                );
                break;
            }
            _ => {}
        }
    }
    assert_eq!(final_size, Some(9));
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_unaccepted_listing_cannot_replace_the_actual_visible_snapshot() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path();
    fs::create_dir_all(base.join("a/folder")).unwrap();
    fs::create_dir(base.join("b")).unwrap();
    fs::write(base.join("a/folder/file"), [1u8; 9]).unwrap();
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(midnight_platform::LocalFileSystem::new(Some(base.into()))),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, &base.join("a")))
        .await
        .unwrap();
    let entries = listing(&mut events, PaneId::Left, 1).await;
    commands
        .send(Command::PaneCommitted(midnight_core::VisibleDirectory {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: base.join("a"),
            entries,
        }))
        .await
        .unwrap();
    commands
        .send(navigate(PaneId::Left, 2, &base.join("b")))
        .await
        .unwrap();
    // B finished on the worker but was never accepted by the UI.
    listing(&mut events, PaneId::Left, 2).await;
    commands
        .send(navigate(PaneId::Left, 3, &base.join("missing")))
        .await
        .unwrap();
    loop {
        if let Event::ListingFailed {
            navigation: NavigationId(3),
            ..
        } = next(&mut events).await
        {
            break;
        }
    }
    commands.send(scan(PaneId::Left, 1, 1)).await.unwrap();
    let mut total = None;
    loop {
        match next(&mut events).await {
            Event::FolderSizeUpdated { size, .. } if size.finished => total = Some(size.bytes),
            Event::FolderSizesFinished { cancelled, .. } => {
                assert!(!cancelled);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(total, Some(9));
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn size_sort_uses_measured_folders_and_preserves_entry_identities() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path();
    for (name, size) in [("a-large", 19), ("z-small", 3)] {
        fs::create_dir(base.join(name)).unwrap();
        fs::write(base.join(name).join("file"), vec![0; size]).unwrap();
    }
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(midnight_platform::LocalFileSystem::new(Some(base.into()))),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, base))
        .await
        .unwrap();
    let original = listing(&mut events, PaneId::Left, 1).await;
    commands.send(scan(PaneId::Left, 1, 1)).await.unwrap();
    loop {
        if let Event::FolderSizesFinished {
            cancelled: false, ..
        } = next(&mut events).await
        {
            break;
        }
    }
    let mut visible = (NavigationId(1), original.clone());
    for (generation, descending, expected) in [(2, false, [3, 19]), (3, true, [19, 3])] {
        commands
            .send(Command::Resort {
                pane: PaneId::Left,
                navigation: NavigationId(generation),
                path: base.into(),
                sort: SortSpec {
                    column: midnight_core::SortColumn::Size,
                    descending,
                },
                source: Some(midnight_core::VisibleDirectory {
                    pane: PaneId::Left,
                    navigation: visible.0,
                    path: base.into(),
                    entries: visible.1.clone(),
                }),
            })
            .await
            .unwrap();
        let sorted = listing(&mut events, PaneId::Left, generation).await;
        assert_eq!(
            sorted
                .iter()
                .map(|entry| entry.metadata.size.unwrap())
                .collect::<Vec<_>>(),
            expected
        );
        for entry in sorted.iter() {
            assert_eq!(
                original
                    .iter()
                    .find(|old| old.path == entry.path)
                    .unwrap()
                    .id,
                entry.id
            );
        }
        visible = (NavigationId(generation), sorted);
    }
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}

struct ControlledTree {
    entered: Notify,
    exited: Notify,
    reads: AtomicUsize,
    root_reads: AtomicUsize,
    recycle_ids: bool,
    completed_first: bool,
    token: Mutex<Option<Cancellation>>,
    released: (Mutex<bool>, Condvar),
    cooperative: bool,
    block_refresh: bool,
    refresh_entered: Notify,
    refresh_released: (Mutex<bool>, Condvar),
}

impl ControlledTree {
    fn new(cooperative: bool) -> Self {
        Self {
            entered: Notify::new(),
            exited: Notify::new(),
            reads: AtomicUsize::new(0),
            root_reads: AtomicUsize::new(0),
            recycle_ids: false,
            completed_first: false,
            token: Mutex::new(None),
            released: (Mutex::new(false), Condvar::new()),
            cooperative,
            block_refresh: false,
            refresh_entered: Notify::new(),
            refresh_released: (Mutex::new(false), Condvar::new()),
        }
    }

    fn release(&self) {
        *self.released.0.lock().unwrap() = true;
        self.released.1.notify_all();
    }

    fn was_cancelled(&self) -> bool {
        self.token.lock().unwrap().as_ref().unwrap().is_cancelled()
    }

    fn release_refresh(&self) {
        *self.refresh_released.0.lock().unwrap() = true;
        self.refresh_released.1.notify_all();
    }
}

struct ReleaseOnDrop(Arc<ControlledTree>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
        self.0.release_refresh();
    }
}

impl DirectoryService for ControlledTree {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if path == Path::new("active") {
            let refreshed = self.root_reads.fetch_add(1, Ordering::SeqCst) > 0;
            if refreshed && self.block_refresh {
                self.refresh_entered.notify_one();
                let mut released = self.refresh_released.0.lock().unwrap();
                while !*released && !cancel.is_cancelled() {
                    released = self
                        .refresh_released
                        .1
                        .wait_timeout(released, Duration::from_millis(5))
                        .unwrap()
                        .0;
                }
            }
            if self.completed_first {
                for (id, name) in [(3, "a-small"), (4, "b-large")] {
                    emit(Entry::new(
                        EntryId(id),
                        path.join(name),
                        EntryKind::Directory,
                    ));
                }
            }
            if self.recycle_ids && refreshed {
                emit(Entry::new(
                    EntryId(1),
                    path.join("new"),
                    EntryKind::Directory,
                ));
            }
            emit(Entry::new(
                EntryId(if self.recycle_ids && refreshed { 99 } else { 1 }),
                path.join("blocked"),
                EntryKind::Directory,
            ));
        } else if path == Path::new("active/blocked") {
            *self.token.lock().unwrap() = Some(cancel.clone());
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            let mut released = self.released.0.lock().unwrap();
            while !*released && (!self.cooperative || !cancel.is_cancelled()) {
                released = self
                    .released
                    .1
                    .wait_timeout(released, Duration::from_millis(5))
                    .unwrap()
                    .0;
            }
            drop(released);
            if !cancel.is_cancelled() {
                emit(Entry::new(
                    EntryId(2),
                    path.join("file.txt"),
                    EntryKind::File,
                ));
            }
            self.exited.notify_one();
        } else {
            emit(Entry::new(
                EntryId(20),
                path.join("file.txt"),
                EntryKind::File,
            ));
        }
        Ok(())
    }

    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        let length = if path.parent() == Some(Path::new("active/a-small")) {
            90
        } else if path.parent() == Some(Path::new("active/b-large")) {
            3
        } else {
            42
        };
        Ok(EntryMetadata {
            size: (path.extension().is_some()).then_some(length),
            modified: None,
            ..EntryMetadata::default()
        })
    }
}

impl VolumeService for ControlledTree {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}

impl StorageService for ControlledTree {
    fn storage_kind(&self, _: &Path) -> Result<midnight_core::StorageKind, FsError> {
        // This scenario needs completed sibling roots while one root is blocked.
        // Independent SSD slots make that possible without depending on the
        // order in which scoped workers acquire the conservative I/O gate.
        Ok(if self.completed_first {
            midnight_core::StorageKind::LocalSsd
        } else {
            midnight_core::StorageKind::Unknown
        })
    }

    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            available: 50,
            total: 100,
        })
    }
}

async fn entered(tree: &ControlledTree) {
    tokio::time::timeout(Duration::from_secs(5), tree.entered.notified())
        .await
        .expect("Recursive size worker must enter controlled directory");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_scan_supersedes_a_blocked_individual_scan_before_replacing_its_cache() {
    let filesystem = Arc::new(ControlledTree::new(false));
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    let entries = listing(&mut events, PaneId::Left, 1).await;
    commands
        .send(Command::ComputeEntrySize {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan: FolderSizeScanId(1),
            entry: entries[0].id,
        })
        .await
        .unwrap();
    entered(&filesystem).await;
    // Consume the initial estimate so the old scan's next update below must be
    // its final cancelled partial, delivered after the blocked read returns.
    loop {
        if let Event::FolderSizeUpdated {
            pane,
            navigation,
            scan,
            entry,
            size,
        } = next(&mut events).await
        {
            assert_eq!(
                (pane, navigation, scan, entry),
                (
                    PaneId::Left,
                    NavigationId(1),
                    FolderSizeScanId(1),
                    entries[0].id,
                )
            );
            assert!(!size.finished);
            assert_eq!(size.bytes, 0);
            break;
        }
    }
    let old_token = filesystem.token.lock().unwrap().clone().unwrap();
    commands.send(scan(PaneId::Left, 1, 2)).await.unwrap();
    // The next command confirms full-scan dispatch without waiting for its blocked I/O.
    commands
        .send(navigate(PaneId::Right, 1, Path::new("other")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Right, 1).await;
    assert!(old_token.is_cancelled());
    filesystem.release();
    let mut final_size = None;
    let mut cancelled_partial = false;
    let mut replacement_finished = false;
    while !cancelled_partial || !replacement_finished {
        match next(&mut events).await {
            Event::FolderSizeUpdated {
                pane,
                navigation,
                scan,
                entry,
                size,
            } => {
                assert_eq!(
                    (pane, navigation, entry),
                    (PaneId::Left, NavigationId(1), entries[0].id)
                );
                assert_eq!(size.errors, 0, "cancellation is not a read error");
                assert!(size.warnings.is_none());
                match scan {
                    FolderSizeScanId(1) => {
                        assert!(!size.finished, "superseded scan cannot claim completion");
                        assert_eq!(size.bytes, 0, "cancelled blocked read emitted no entries");
                        cancelled_partial = true;
                    }
                    FolderSizeScanId(2) => {
                        if size.finished {
                            final_size = Some(size.bytes);
                        }
                    }
                    other => panic!("Unexpected size scan: {other:?}"),
                }
            }
            Event::FolderSizesFinished {
                pane,
                navigation,
                scan,
                cancelled,
            } => {
                assert_eq!((pane, navigation), (PaneId::Left, NavigationId(1)));
                assert_eq!(scan, FolderSizeScanId(2));
                assert!(!cancelled);
                replacement_finished = true;
            }
            _ => {}
        }
    }
    assert_eq!(final_size, Some(42));
    // Both workers have now published their final values. A later explicit size
    // sort must use the replacement cache, never the old scan's zero-byte partial.
    commands
        .send(Command::Resort {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: "active".into(),
            sort: SortSpec {
                column: midnight_core::SortColumn::Size,
                descending: false,
            },
            source: Some(midnight_core::VisibleDirectory {
                pane: PaneId::Left,
                navigation: NavigationId(1),
                path: "active".into(),
                entries: entries.clone(),
            }),
        })
        .await
        .unwrap();
    let sorted = listing(&mut events, PaneId::Left, 2).await;
    assert_eq!(sorted.len(), 1);
    assert_eq!(sorted[0].id, entries[0].id);
    assert_eq!(sorted[0].metadata.size, Some(42));
    assert_eq!(
        filesystem.reads.load(Ordering::SeqCst),
        2,
        "sorting must reuse the replacement result without rescanning"
    );
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn individual_scan_finishing_during_refresh_remains_available_to_explicit_size_sort() {
    let mut tree = ControlledTree::new(true);
    tree.block_refresh = true;
    let filesystem = Arc::new(tree);
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    let entries = listing(&mut events, PaneId::Left, 1).await;
    commands
        .send(Command::ComputeEntrySize {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan: FolderSizeScanId(1),
            entry: entries[0].id,
        })
        .await
        .unwrap();
    entered(&filesystem).await;
    commands
        .send(Command::Refresh {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: "active".into(),
            sort: SortSpec::default(),
            scan: None,
        })
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(5),
        filesystem.refresh_entered.notified(),
    )
    .await
    .unwrap();
    filesystem.release();
    complete_scan(&mut events, 1, false).await;
    filesystem.release_refresh();
    let refreshed = listing(&mut events, PaneId::Left, 2).await;
    assert_eq!(refreshed[0].metadata.size, Some(42));
    commands
        .send(Command::Resort {
            pane: PaneId::Left,
            navigation: NavigationId(3),
            path: "active".into(),
            sort: SortSpec {
                column: midnight_core::SortColumn::Size,
                descending: false,
            },
            source: Some(midnight_core::VisibleDirectory {
                pane: PaneId::Left,
                navigation: NavigationId(2),
                path: "active".into(),
                entries: refreshed,
            }),
        })
        .await
        .unwrap();
    let sorted = listing(&mut events, PaneId::Left, 3).await;
    assert_eq!(sorted[0].metadata.size, Some(42));
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_honors_target_continuation_until_the_ui_retires_its_completion() {
    let filesystem = Arc::new(ControlledTree::new(true));
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    let entries = listing(&mut events, PaneId::Left, 1).await;
    commands
        .send(Command::ComputeEntrySize {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan: FolderSizeScanId(1),
            entry: entries[0].id,
        })
        .await
        .unwrap();
    entered(&filesystem).await;
    filesystem.release();
    complete_scan(&mut events, 1, false).await;
    // Consume the backend completion without applying it to a presenter: the UI still
    // names the request while two consecutive refreshes are dispatched.
    for navigation in [2, 3, 4] {
        commands
            .send(Command::Refresh {
                pane: PaneId::Left,
                navigation: NavigationId(navigation),
                path: "active".into(),
                sort: SortSpec {
                    column: midnight_core::SortColumn::Size,
                    descending: false,
                },
                scan: (navigation < 4).then_some(FolderSizeScanId(1)),
            })
            .await
            .unwrap();
        let refreshed = listing(&mut events, PaneId::Left, navigation).await;
        assert_eq!(refreshed[0].metadata.size, (navigation < 4).then_some(42));
    }
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

async fn complete_scan(events: &mut mpsc::Receiver<Event>, expected_scan: u64, cancelled: bool) {
    loop {
        if let Event::FolderSizesFinished {
            scan: FolderSizeScanId(scan),
            cancelled: actual,
            ..
        } = next(events).await
        {
            assert_eq!(scan, expected_scan);
            assert_eq!(actual, cancelled);
            return;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_scan_survives_repeated_resorts_without_restarting_or_losing_results() {
    let filesystem = Arc::new(ControlledTree::new(true));
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    let original = listing(&mut events, PaneId::Left, 1).await;
    commands.send(scan(PaneId::Left, 1, 1)).await.unwrap();
    entered(&filesystem).await;

    let mut visible = (NavigationId(1), original.clone());
    for generation in 2..=4 {
        commands
            .send(Command::Resort {
                pane: PaneId::Left,
                navigation: NavigationId(generation),
                path: "active".into(),
                sort: SortSpec {
                    column: midnight_core::SortColumn::Name,
                    descending: generation % 2 == 0,
                },
                source: Some(midnight_core::VisibleDirectory {
                    pane: PaneId::Left,
                    navigation: visible.0,
                    path: "active".into(),
                    entries: visible.1.clone(),
                }),
            })
            .await
            .unwrap();
        let sorted = listing(&mut events, PaneId::Left, generation).await;
        assert_eq!(sorted[0].id, original[0].id);
        assert!(
            !filesystem.was_cancelled(),
            "sorting is not scan cancellation"
        );
        assert_eq!(filesystem.reads.load(Ordering::SeqCst), 1);
        visible = (NavigationId(generation), sorted);
    }

    filesystem.release();
    let mut final_size = None;
    loop {
        match next(&mut events).await {
            Event::FolderSizeUpdated {
                navigation,
                scan,
                entry,
                size,
                ..
            } if size.finished => {
                assert_eq!(navigation, NavigationId(1));
                assert_eq!(scan, FolderSizeScanId(1));
                assert_eq!(entry, original[0].id);
                final_size = Some(size.bytes);
            }
            Event::FolderSizesFinished { cancelled, .. } => {
                assert!(!cancelled);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(final_size, Some(42));
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_continues_live_scan_and_resort_uses_raw_paths_after_id_recycling() {
    let mut tree = ControlledTree::new(true);
    tree.recycle_ids = true;
    let filesystem = Arc::new(tree);
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 1).await;
    commands.send(scan(PaneId::Left, 1, 1)).await.unwrap();
    entered(&filesystem).await;
    commands
        .send(Command::Refresh {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: "active".into(),
            sort: SortSpec::default(),
            scan: Some(FolderSizeScanId(1)),
        })
        .await
        .unwrap();
    let refreshed = listing(&mut events, PaneId::Left, 2).await;
    assert!(
        refreshed
            .iter()
            .any(|entry| entry.id == EntryId(1) && entry.path == Path::new("active/new"))
    );
    assert!(
        refreshed
            .iter()
            .any(|entry| entry.id == EntryId(99) && entry.path == Path::new("active/blocked"))
    );
    assert!(!filesystem.was_cancelled());
    assert_eq!(filesystem.reads.load(Ordering::SeqCst), 1);
    filesystem.release();
    complete_scan(&mut events, 1, false).await;
    commands
        .send(Command::Resort {
            pane: PaneId::Left,
            navigation: NavigationId(3),
            path: "active".into(),
            sort: SortSpec {
                column: midnight_core::SortColumn::Size,
                descending: false,
            },
            source: Some(midnight_core::VisibleDirectory {
                pane: PaneId::Left,
                navigation: NavigationId(2),
                path: "active".into(),
                entries: refreshed,
            }),
        })
        .await
        .unwrap();
    let sorted = listing(&mut events, PaneId::Left, 3).await;
    let old_root = sorted
        .iter()
        .find(|entry| entry.path == Path::new("active/blocked"))
        .unwrap();
    let new_root = sorted
        .iter()
        .find(|entry| entry.path == Path::new("active/new"))
        .unwrap();
    assert_eq!(old_root.metadata.size, Some(42));
    assert_eq!(
        new_root.metadata.size, None,
        "recycled IDs must not inherit another folder's size"
    );
    commands
        .send(Command::Refresh {
            pane: PaneId::Left,
            navigation: NavigationId(4),
            path: "active".into(),
            sort: SortSpec::default(),
            scan: None,
        })
        .await
        .unwrap();
    let fresh = listing(&mut events, PaneId::Left, 4).await;
    commands
        .send(Command::Resort {
            pane: PaneId::Left,
            navigation: NavigationId(5),
            path: "active".into(),
            sort: SortSpec {
                column: midnight_core::SortColumn::Size,
                descending: false,
            },
            source: Some(midnight_core::VisibleDirectory {
                pane: PaneId::Left,
                navigation: NavigationId(4),
                path: "active".into(),
                entries: fresh,
            }),
        })
        .await
        .unwrap();
    let cleared = listing(&mut events, PaneId::Left, 5).await;
    assert!(
        cleared.iter().all(|entry| entry.metadata.size.is_none()),
        "a new refresh after completion must not resurrect old finished estimates"
    );
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_path_new_context_navigation_still_cancels_the_old_scan() {
    let filesystem = Arc::new(ControlledTree::new(true));
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 1).await;
    commands.send(scan(PaneId::Left, 1, 1)).await.unwrap();
    entered(&filesystem).await;
    commands
        .send(navigate(PaneId::Left, 2, Path::new("active")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 2).await;
    assert!(
        filesystem.was_cancelled(),
        "a new tab is not an in-place refresh"
    );
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_applies_live_completed_totals_before_sorting_by_size() {
    let mut tree = ControlledTree::new(true);
    tree.completed_first = true;
    let filesystem = Arc::new(tree);
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 1).await;
    commands.send(scan(PaneId::Left, 1, 1)).await.unwrap();
    entered(&filesystem).await;
    let mut completed = std::collections::HashSet::new();
    while completed.len() < 2 {
        if let Event::FolderSizeUpdated {
            pane: PaneId::Left,
            scan: FolderSizeScanId(1),
            entry,
            size,
            ..
        } = next(&mut events).await
            && size.finished
            && matches!(entry, EntryId(3) | EntryId(4))
        {
            // The cache is written before this event is published. Entering
            // the blocked sibling alone says nothing about these completions.
            completed.insert(entry);
        }
    }
    commands
        .send(Command::Refresh {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: "active".into(),
            sort: SortSpec {
                column: midnight_core::SortColumn::Size,
                descending: false,
            },
            scan: Some(FolderSizeScanId(1)),
        })
        .await
        .unwrap();
    let refreshed = listing(&mut events, PaneId::Left, 2).await;
    let known = refreshed
        .iter()
        .filter_map(|entry| entry.metadata.size)
        .collect::<Vec<_>>();
    assert_eq!(
        known,
        [3, 90],
        "refresh must sort using the live path cache"
    );
    assert!(!filesystem.was_cancelled());
    filesystem.release();
    complete_scan(&mut events, 1, false).await;
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resort_uses_exact_displayed_snapshot_before_visible_acknowledgment() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path();
    fs::write(base.join("old.txt"), "old").unwrap();
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(midnight_platform::LocalFileSystem::new(Some(base.into()))),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, base))
        .await
        .unwrap();
    let first = listing(&mut events, PaneId::Left, 1).await;
    commands
        .send(Command::PaneCommitted(midnight_core::VisibleDirectory {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: base.into(),
            entries: first,
        }))
        .await
        .unwrap();
    fs::remove_file(base.join("old.txt")).unwrap();
    fs::write(base.join("displayed.txt"), "current").unwrap();
    commands
        .send(Command::Refresh {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: base.into(),
            sort: SortSpec::default(),
            scan: None,
        })
        .await
        .unwrap();
    let displayed = listing(&mut events, PaneId::Left, 2).await;
    // No PaneCommitted(2): an exhausted UI command queue can delay this acknowledgment.
    commands
        .send(Command::Resort {
            pane: PaneId::Left,
            navigation: NavigationId(3),
            path: base.into(),
            sort: SortSpec::default(),
            source: Some(midnight_core::VisibleDirectory {
                pane: PaneId::Left,
                navigation: NavigationId(2),
                path: base.into(),
                entries: displayed,
            }),
        })
        .await
        .unwrap();
    let sorted = listing(&mut events, PaneId::Left, 3).await;
    assert_eq!(sorted.len(), 1);
    assert_eq!(sorted[0].path, base.join("displayed.txt"));
    fs::write(base.join("unaccepted.txt"), "not committed to UI").unwrap();
    commands
        .send(Command::Refresh {
            pane: PaneId::Left,
            navigation: NavigationId(4),
            path: base.into(),
            sort: SortSpec::default(),
            scan: None,
        })
        .await
        .unwrap();
    assert_eq!(listing(&mut events, PaneId::Left, 4).await.len(), 2);
    // Neither visible(1) nor newest committed(4) is what the UI is displaying(3).
    commands
        .send(Command::Resort {
            pane: PaneId::Left,
            navigation: NavigationId(5),
            path: base.into(),
            sort: SortSpec::default(),
            source: Some(midnight_core::VisibleDirectory {
                pane: PaneId::Left,
                navigation: NavigationId(3),
                path: base.into(),
                entries: sorted,
            }),
        })
        .await
        .unwrap();
    let still_displayed = listing(&mut events, PaneId::Left, 5).await;
    assert_eq!(still_displayed.len(), 1);
    assert_eq!(still_displayed[0].path, base.join("displayed.txt"));
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_scan_keeps_other_pane_navigation_responsive_and_can_be_cancelled() {
    let filesystem = Arc::new(ControlledTree::new(true));
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 1).await;
    commands.send(scan(PaneId::Left, 1, 1)).await.unwrap();
    entered(&filesystem).await;
    commands
        .send(navigate(PaneId::Right, 1, Path::new("other")))
        .await
        .unwrap();
    assert_eq!(listing(&mut events, PaneId::Right, 1).await.len(), 1);
    commands
        .send(Command::CancelFolderSizes {
            pane: PaneId::Left,
            scan: FolderSizeScanId(1),
        })
        .await
        .unwrap();
    complete_scan(&mut events, 1, true).await;
    assert!(filesystem.was_cancelled());
    filesystem.release();
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_for_another_scan_id_does_not_cancel_current_scan() {
    let filesystem = Arc::new(ControlledTree::new(true));
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 1).await;
    commands.send(scan(PaneId::Left, 1, 1)).await.unwrap();
    entered(&filesystem).await;
    commands
        .send(Command::CancelFolderSizes {
            pane: PaneId::Left,
            scan: FolderSizeScanId(999),
        })
        .await
        .unwrap();
    // A subsequent listing is also a barrier proving the earlier command was handled.
    commands
        .send(navigate(PaneId::Right, 1, Path::new("other")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Right, 1).await;
    assert!(!filesystem.was_cancelled());
    filesystem.release();
    complete_scan(&mut events, 1, false).await;
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacing_navigation_cancels_old_scan_and_suppresses_its_completion() {
    let filesystem = Arc::new(ControlledTree::new(false));
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 1).await;
    commands.send(scan(PaneId::Left, 1, 1)).await.unwrap();
    entered(&filesystem).await;
    commands
        .send(navigate(PaneId::Left, 2, Path::new("other")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 2).await;
    assert!(filesystem.was_cancelled());
    filesystem.release();
    commands.send(scan(PaneId::Left, 2, 2)).await.unwrap();
    loop {
        match next(&mut events).await {
            Event::FolderSizeUpdated {
                scan: FolderSizeScanId(1),
                size,
                ..
            } if size.finished => panic!("Cancelled old scan emitted a final result"),
            Event::FolderSizesFinished {
                scan: FolderSizeScanId(1),
                ..
            } => panic!("Cancelled old scan emitted completion after new navigation"),
            Event::FolderSizesFinished {
                scan: FolderSizeScanId(2),
                cancelled,
                ..
            } => {
                assert!(!cancelled);
                break;
            }
            _ => {}
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_navigation_generation_is_rejected_without_recursing() {
    let filesystem = Arc::new(ControlledTree::new(true));
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 1).await;
    commands.send(scan(PaneId::Left, 0, 1)).await.unwrap();
    complete_scan(&mut events, 1, true).await;
    assert_eq!(filesystem.reads.load(Ordering::SeqCst), 0);
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replacement_scan_waits_for_previous_os_worker_to_release_its_budget() {
    let filesystem = Arc::new(ControlledTree::new(false));
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 1).await;
    commands.send(scan(PaneId::Left, 1, 1)).await.unwrap();
    entered(&filesystem).await;
    commands.send(scan(PaneId::Left, 1, 2)).await.unwrap();
    commands
        .send(navigate(PaneId::Right, 1, Path::new("other")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Right, 1).await;
    assert!(filesystem.was_cancelled());
    assert_eq!(filesystem.reads.load(Ordering::SeqCst), 1);
    filesystem.release();
    complete_scan(&mut events, 2, false).await;
    assert_eq!(filesystem.reads.load(Ordering::SeqCst), 2);
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_compute_requests_do_not_cancel_or_replace_a_current_scan() {
    let filesystem = Arc::new(ControlledTree::new(true));
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 1).await;
    commands.send(scan(PaneId::Left, 1, 5)).await.unwrap();
    entered(&filesystem).await;
    commands.send(scan(PaneId::Left, 0, 6)).await.unwrap();
    complete_scan(&mut events, 6, true).await;
    assert!(!filesystem.was_cancelled());
    commands.send(scan(PaneId::Left, 1, 4)).await.unwrap();
    complete_scan(&mut events, 4, true).await;
    assert!(!filesystem.was_cancelled());
    assert_eq!(filesystem.reads.load(Ordering::SeqCst), 1);
    filesystem.release();
    complete_scan(&mut events, 5, false).await;
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_compute_command_cannot_falsely_finish_the_active_scan() {
    let filesystem = Arc::new(ControlledTree::new(true));
    let _release = ReleaseOnDrop(filesystem.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            filesystem.clone(),
            sender,
        )
        .run(receiver),
    );
    commands
        .send(navigate(PaneId::Left, 1, Path::new("active")))
        .await
        .unwrap();
    listing(&mut events, PaneId::Left, 1).await;
    commands.send(scan(PaneId::Left, 1, 5)).await.unwrap();
    entered(&filesystem).await;
    commands.send(scan(PaneId::Left, 1, 5)).await.unwrap();
    commands
        .send(navigate(PaneId::Right, 1, Path::new("other")))
        .await
        .unwrap();
    loop {
        match next(&mut events).await {
            Event::FolderSizesFinished {
                scan: FolderSizeScanId(5),
                ..
            } => panic!("Duplicate scan command falsely finished the active scan"),
            Event::ListingFinished {
                pane: PaneId::Right,
                ..
            } => break,
            _ => {}
        }
    }
    assert!(!filesystem.was_cancelled());
    assert_eq!(filesystem.reads.load(Ordering::SeqCst), 1);
    filesystem.release();
    complete_scan(&mut events, 5, false).await;
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}
