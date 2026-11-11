//! Captured selection limits backend I/O and preserves unrelated sorting totals.
use midnight_core::*;
use midnight_engine::Engine;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

struct Tree {
    reads: Mutex<Vec<PathBuf>>,
    selected: AtomicU64,
    unselected: AtomicU64,
}

impl DirectoryService for Tree {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.reads.lock().unwrap().push(path.to_owned());
        if path == Path::new("active") {
            for (id, name, kind) in [
                (1, "selected-資料", EntryKind::Directory),
                (2, "unselected", EntryKind::Directory),
                (3, "ordinary.txt", EntryKind::File),
                (4, "link", EntryKind::DirectoryLink),
            ] {
                emit(Entry::new(EntryId(id), path.join(name), kind));
            }
        } else if path == Path::new("active/selected-資料")
            || path == Path::new("active/unselected")
        {
            emit(Entry::new(
                EntryId(10),
                path.join("payload.txt"),
                EntryKind::File,
            ));
        } else {
            panic!(
                "must not read an unselected/link target: {}",
                path.display()
            );
        }
        Ok(())
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        let bytes = if path.parent() == Some(Path::new("active/selected-資料")) {
            self.selected.load(Ordering::Relaxed)
        } else if path.parent() == Some(Path::new("active/unselected")) {
            self.unselected.load(Ordering::Relaxed)
        } else {
            5
        };
        Ok(EntryMetadata {
            size: Some(bytes),
            ..Default::default()
        })
    }
}
impl VolumeService for Tree {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for Tree {
    fn storage_kind(&self, _: &Path) -> Result<StorageKind, FsError> {
        Ok(StorageKind::Unknown)
    }
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            available: 50,
            total: 100,
        })
    }
}

async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn listing(events: &mut mpsc::Receiver<Event>, generation: u64) -> Arc<[Entry]> {
    loop {
        if let Event::ListingFinished {
            navigation,
            entries,
            ..
        } = next(events).await
            && navigation == NavigationId(generation)
        {
            return entries;
        }
    }
}
async fn sizes(events: &mut mpsc::Receiver<Event>, expected: u64) -> HashMap<EntryId, u64> {
    let mut totals = HashMap::new();
    loop {
        match next(events).await {
            Event::FolderSizeUpdated {
                scan, entry, size, ..
            } if scan == FolderSizeScanId(expected) => {
                if size.finished {
                    totals.insert(entry, size.bytes);
                }
            }
            Event::FolderSizesFinished {
                scan, cancelled, ..
            } if scan == FolderSizeScanId(expected) => {
                assert!(!cancelled);
                return totals;
            }
            _ => {}
        }
    }
}
fn selection(entries: &[Entry], names: &[&str]) -> Arc<SelectionSnapshot> {
    let index = Arc::new(DirectoryIndex::from_entries(entries));
    let mut bits = vec![0u64; entries.len().div_ceil(64)];
    for (position, entry) in entries.iter().enumerate() {
        if names
            .iter()
            .any(|name| entry.path.file_name() == Some(std::ffi::OsStr::new(name)))
        {
            bits[position / 64] |= 1 << (position % 64);
        }
    }
    Arc::new(SelectionSnapshot::from_bits(index, bits).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captured_directories_only_are_read_and_unselected_cached_totals_survive_sort() {
    let tree = Arc::new(Tree {
        reads: Mutex::new(vec![]),
        selected: AtomicU64::new(7),
        unselected: AtomicU64::new(11),
    });
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(Engine::new(tree.clone(), sender).run(receiver));
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: "active".into(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let entries = listing(&mut events, 1).await;
    commands
        .send(Command::ComputeFolderSizes {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan: FolderSizeScanId(1),
            selection: None,
        })
        .await
        .unwrap();
    assert_eq!(
        sizes(&mut events, 1).await.len(),
        3,
        "ordinary directories and the explicitly unsupported directory link"
    );
    tree.reads.lock().unwrap().clear();
    tree.selected.store(9, Ordering::Relaxed);
    tree.unselected.store(99, Ordering::Relaxed);
    commands
        .send(Command::ComputeFolderSizes {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan: FolderSizeScanId(2),
            selection: Some(selection(
                &entries,
                &["selected-資料", "ordinary.txt", "link"],
            )),
        })
        .await
        .unwrap();
    assert_eq!(
        sizes(&mut events, 2).await,
        HashMap::from([(EntryId(1), 9), (EntryId(4), 0)])
    );
    assert_eq!(
        *tree.reads.lock().unwrap(),
        vec![PathBuf::from("active/selected-資料")]
    );
    commands
        .send(Command::Resort {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: "active".into(),
            sort: SortSpec {
                column: SortColumn::Size,
                descending: false,
            },
            source: Some(VisibleDirectory {
                pane: PaneId::Left,
                navigation: NavigationId(1),
                path: "active".into(),
                entries,
            }),
        })
        .await
        .unwrap();
    let sorted = listing(&mut events, 2).await;
    assert_eq!(
        sorted
            .iter()
            .find(|entry| entry.path == Path::new("active/selected-資料"))
            .unwrap()
            .metadata
            .size,
        Some(9)
    );
    assert_eq!(
        sorted
            .iter()
            .find(|entry| entry.path == Path::new("active/unselected"))
            .unwrap()
            .metadata
            .size,
        Some(11)
    );
    for (scan, names) in [(3, vec!["ordinary.txt"]), (4, vec!["link"]), (5, vec![])] {
        tree.reads.lock().unwrap().clear();
        commands
            .send(Command::ComputeFolderSizes {
                pane: PaneId::Left,
                navigation: NavigationId(2),
                scan: FolderSizeScanId(scan),
                selection: Some(selection(&sorted, &names)),
            })
            .await
            .unwrap();
        let expected = if names == ["link"] {
            HashMap::from([(EntryId(4), 0)])
        } else {
            HashMap::new()
        };
        assert_eq!(sizes(&mut events, scan).await, expected);
        assert!(
            tree.reads.lock().unwrap().is_empty(),
            "files, unsupported links and empty explicit scope must not fall back to all roots"
        );
    }
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

async fn summary(events: &mut mpsc::Receiver<Event>, id: u64) -> SelectedFolderSizes {
    loop {
        if let Event::SelectedFolderSizes {
            request, summary, ..
        } = next(events).await
            && request == SelectionSizeId(id)
        {
            return summary;
        }
    }
}

async fn scan_summary(
    events: &mut mpsc::Receiver<Event>,
    scan: u64,
    request_id: u64,
    expected_complete: usize,
    expected_bytes: u64,
) -> SelectedFolderSizes {
    let mut latest = None;
    let mut finished = false;
    loop {
        match next(events).await {
            Event::SelectedFolderSizes {
                request, summary, ..
            } if request == SelectionSizeId(request_id) => latest = Some(summary),
            Event::FolderSizesFinished {
                scan: actual,
                cancelled,
                ..
            } if actual == FolderSizeScanId(scan) => {
                assert!(!cancelled);
                finished = true;
            }
            _ => {}
        }
        if finished
            && let Some(summary) = latest
            && summary.complete == expected_complete
            && summary.complete_bytes == Some(expected_bytes)
        {
            return summary;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_status_reads_cached_totals_and_tracks_global_and_individual_scans() {
    let tree = Arc::new(Tree {
        reads: Mutex::new(vec![]),
        selected: AtomicU64::new(7),
        unselected: AtomicU64::new(11),
    });
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(Engine::new(tree.clone(), sender).run(receiver));
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: "active".into(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let entries = listing(&mut events, 1).await;
    let request = |id, names: &[&str]| {
        Command::SummarizeSelectedFolders(SelectionSizeRequest {
            id: SelectionSizeId(id),
            pane: PaneId::Left,
            navigation: NavigationId(1),
            revision: SelectionRevision(id),
            source_navigation: NavigationId(1),
            location: "active".into(),
            selection: selection(&entries, names),
        })
    };
    commands
        .send(request(1, &["selected-資料", "unselected", "link"]))
        .await
        .unwrap();
    let unknown = summary(&mut events, 1).await;
    assert_eq!(
        (unknown.folders, unknown.complete, unknown.partial),
        (3, 0, 0)
    );
    commands
        .send(Command::ComputeFolderSizes {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan: FolderSizeScanId(1),
            selection: None,
        })
        .await
        .unwrap();
    let all = scan_summary(&mut events, 1, 1, 2, 18).await;
    assert_eq!(
        (all.folders, all.complete, all.complete_bytes, all.partial),
        (3, 2, Some(18), 1)
    );
    commands.send(request(2, &["unselected"])).await.unwrap();
    assert_eq!(summary(&mut events, 2).await.complete_bytes, Some(11));
    tree.unselected.store(99, Ordering::Relaxed);
    commands
        .send(Command::ComputeEntrySize {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan: FolderSizeScanId(2),
            entry: EntryId(2),
        })
        .await
        .unwrap();
    assert_eq!(
        scan_summary(&mut events, 2, 2, 1, 99).await.complete_bytes,
        Some(99)
    );
    tree.selected.store(9, Ordering::Relaxed);
    commands
        .send(Command::ComputeFolderSizes {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan: FolderSizeScanId(3),
            selection: Some(selection(&entries, &["selected-資料"])),
        })
        .await
        .unwrap();
    assert_eq!(
        scan_summary(&mut events, 3, 2, 1, 99).await.complete_bytes,
        Some(99)
    );
    commands
        .send(request(3, &["selected-資料", "link"]))
        .await
        .unwrap();
    let current = summary(&mut events, 3).await;
    assert_eq!(
        (current.folders, current.complete, current.complete_bytes),
        (2, 1, Some(9))
    );
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

async fn reject_native_scan_in_archive(single: bool) {
    let tree = Arc::new(Tree {
        reads: Mutex::new(vec![]),
        selected: AtomicU64::new(7),
        unselected: AtomicU64::new(11),
    });
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(Engine::new(tree.clone(), sender).run(receiver));
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: "active".into(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    listing(&mut events, 1).await;
    commands
        .send(Command::ArchiveNavigate {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            sort: SortSpec::default(),
            action: ArchiveAction::List(ArchiveLocation::unavailable(ArchiveAddress {
                format: None,
                provider: "nonexistent-test-provider".into(),
                container: std::env::current_dir().unwrap().join("absent-fixture.zip"),
                source_version: vec![1],
                components: vec![],
            })),
        })
        .await
        .unwrap();
    loop {
        if let Event::ArchiveListed {
            navigation: NavigationId(2),
            result,
            ..
        } = next(&mut events).await
        {
            assert!(result.is_err());
            break;
        }
    }
    tree.reads.lock().unwrap().clear();
    let command = if single {
        Command::ComputeEntrySize {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan: FolderSizeScanId(1),
            entry: EntryId(1),
        }
    } else {
        Command::ComputeFolderSizes {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan: FolderSizeScanId(1),
            selection: None,
        }
    };
    commands.send(command).await.unwrap();
    loop {
        if let Event::FolderSizesFinished {
            scan: FolderSizeScanId(1),
            cancelled,
            ..
        } = next(&mut events).await
        {
            assert!(
                cancelled,
                "a stale native request must be rejected in a virtual pane"
            );
            break;
        }
    }
    assert!(tree.reads.lock().unwrap().is_empty());
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejects_late_native_full_size_scan_in_unavailable_archive() {
    reject_native_scan_in_archive(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejects_late_native_entry_size_scan_in_unavailable_archive() {
    reject_native_scan_in_archive(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_summary_coalesces_root_completions_and_publishes_the_final_value() {
    let tree = Arc::new(Tree {
        reads: Mutex::new(vec![]),
        selected: AtomicU64::new(7),
        unselected: AtomicU64::new(11),
    });
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(Engine::new(tree, sender).run(receiver));
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: "active".into(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let entries = listing(&mut events, 1).await;
    commands
        .send(Command::SummarizeSelectedFolders(SelectionSizeRequest {
            id: SelectionSizeId(1),
            pane: PaneId::Left,
            navigation: NavigationId(1),
            revision: SelectionRevision(1),
            source_navigation: NavigationId(1),
            location: "active".into(),
            selection: selection(&entries, &["selected-資料", "unselected"]),
        }))
        .await
        .unwrap();
    assert_eq!(summary(&mut events, 1).await.complete, 0);
    let mut last_summary = std::time::Instant::now();
    commands
        .send(Command::ComputeFolderSizes {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            scan: FolderSizeScanId(1),
            selection: None,
        })
        .await
        .unwrap();
    let mut finished = false;
    let mut complete = false;
    while !finished || !complete {
        match next(&mut events).await {
            Event::SelectedFolderSizes { summary, .. } => {
                assert!(
                    last_summary.elapsed() >= Duration::from_millis(200),
                    "root completions must not cause a burst of status publications"
                );
                last_summary = std::time::Instant::now();
                if summary.complete == 2 {
                    assert_eq!(summary.complete_bytes, Some(18));
                    complete = true;
                }
            }
            Event::FolderSizesFinished { cancelled, .. } => {
                assert!(!cancelled);
                finished = true;
            }
            _ => {}
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}
