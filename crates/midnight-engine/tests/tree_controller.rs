//! Exercise tree lifecycle through the production Engine command/event boundary.
use midnight_core::*;
use midnight_engine::Engine;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;

#[path = "tree_refresh/controller.rs"]
mod refresh_tests;

#[path = "tree_refresh/housekeeping.rs"]
mod housekeeping_tests;

struct Files;
impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        emit(Entry::new(
            EntryId(1),
            path.join("東京 folder"),
            EntryKind::Directory,
        ));
        emit(Entry::new(
            EntryId(2),
            path.join("file.txt"),
            EntryKind::File,
        ));
        Ok(())
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("completed rows must be reused")
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for Files {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            total: 100,
            available: 50,
        })
    }
}

async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .expect("engine event timeout")
        .expect("engine event stream")
}

async fn next_tree(events: &mut mpsc::Receiver<Event>) -> TreeEvent {
    loop {
        if let Event::Tree(event) = next(events).await {
            return *event;
        }
    }
}
async fn tree(send: &mpsc::Sender<Command>, command: TreeCommand) {
    send.send(Command::Tree(Box::new(command))).await.unwrap();
}
async fn native_root(
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    navigation: u64,
) -> TreeRootSnapshot {
    native_root_at(
        commands,
        events,
        navigation,
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tree-controller-fixture"),
    )
    .await
}

async fn native_root_at(
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    navigation: u64,
    path: std::path::PathBuf,
) -> TreeRootSnapshot {
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(navigation),
            path,
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    loop {
        if let Event::ListingFinished {
            pane,
            navigation,
            entries,
            index,
        } = next(events).await
        {
            break TreeRootSnapshot {
                pane,
                navigation,
                entries,
                index,
            };
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_native_level_with_many_visible_children_does_not_exhaust_the_queue() {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("東京 parent");
    std::fs::create_dir(&parent).unwrap();
    for i in 0..40 {
        std::fs::create_dir(parent.join(format!("child-{i:02}"))).unwrap();
    }
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let filesystem = midnight_platform::LocalFileSystem::new(None);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(filesystem),
            send,
        )
        .run(receive),
    );
    let root = native_root_at(&commands, &mut events, 1, directory.path().to_owned()).await;
    let publication = enable(&commands, &mut events, root).await;
    let row = publication.view.rows[0].id;
    tree(&commands, TreeCommand::Commit(publication.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Branch {
            view: publication.view,
            request: TreeRequestId(1),
            row,
            action: TreeBranchAction::Expand,
        },
    )
    .await;
    let expanded = loop {
        let event = next_tree(&mut events).await;
        let TreeEvent::Published(publication) = event else {
            panic!("first expansion failed: {event:?}");
        };
        assert!(publication.view.rows.iter().all(|row| row.error.is_none()));
        if publication.view.rows.len() == 41 {
            break publication.view;
        }
    };
    tree(&commands, TreeCommand::Commit(expanded.clone())).await;
    let rows =
        TreeViewport::new(&expanded, expanded.rows.iter().map(|row| row.id).collect()).unwrap();
    tree(
        &commands,
        TreeCommand::Viewport {
            view: expanded,
            rows,
        },
    )
    .await;
    let event = next_tree(&mut events).await;
    assert!(
        matches!(event, TreeEvent::Published(_)),
        "viewport checks failed: {event:?}"
    );
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}
async fn enable(
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    root: TreeRootSnapshot,
) -> TreePublication {
    tree(
        commands,
        TreeCommand::Enable {
            root,
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        },
    )
    .await;
    let TreeEvent::Published(publication) = next_tree(events).await else {
        panic!("publication expected")
    };
    publication
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_refresh_keeps_calculated_sizes_and_frozen_size_order() {
    let directory = tempfile::tempdir().unwrap();
    for (name, bytes) in [("a-small", 7), ("z-large", 17)] {
        let path = directory.path().join(name);
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("leaf"), vec![1; bytes]).unwrap();
    }
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(64);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(midnight_platform::LocalFileSystem::new(None)),
            send,
        )
        .run(receive),
    );
    let root = native_root_at(&commands, &mut events, 1, directory.path().to_owned()).await;
    let publication = enable(&commands, &mut events, root).await;
    tree(&commands, TreeCommand::Commit(publication.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Sizes(TreeSizeCommand::Compute(TreeSizeRequest {
            view: publication.view.clone(),
            request: TreeRequestId(1),
            selection_revision: publication.selection_revision,
            selection: publication.selection.clone(),
            scope: TreeSizeScope::SelectionOrVisible,
        })),
    )
    .await;
    let measured_rows: Vec<_> = publication.view.rows.iter().map(|row| row.id).collect();
    loop {
        if let TreeEvent::Sizes(TreeSizeEvent::Snapshot { sizes }) = next_tree(&mut events).await {
            tree(
                &commands,
                TreeCommand::Sizes(TreeSizeCommand::Acknowledge {
                    stamp: sizes.stamp,
                    version: sizes.version,
                }),
            )
            .await;
            if publication.view.rows.iter().all(|row| {
                sizes.get(row.id).is_some_and(|size| size.finished) && !sizes.running(row.id)
            }) {
                break;
            }
        }
    }
    tree(
        &commands,
        TreeCommand::Reproject(TreeReprojectRequest {
            view: publication.view,
            request: TreeRequestId(2),
            selection_revision: publication.selection_revision,
            selection: publication.selection,
            options: TreeProjectionOptions {
                sort: SortSpec {
                    column: SortColumn::Size,
                    descending: true,
                },
                ..Default::default()
            },
        }),
    )
    .await;
    let sorted = loop {
        if let TreeEvent::Reprojected { publication, .. } = next_tree(&mut events).await {
            break publication;
        }
    };
    assert_eq!(sorted.view.rows[0].entry.name, "z-large");
    tree(&commands, TreeCommand::Commit(sorted.view.clone())).await;
    let target = sorted.view.rows[1].id;
    tree(
        &commands,
        TreeCommand::Branch {
            view: sorted.view.clone(),
            request: TreeRequestId(1),
            row: target,
            action: TreeBranchAction::Refresh,
        },
    )
    .await;
    loop {
        if let TreeEvent::Rejected {
            request: TreeRequestId(1),
            ..
        } = next_tree(&mut events).await
        {
            break;
        }
    }
    tree(
        &commands,
        TreeCommand::Branch {
            view: sorted.view,
            request: TreeRequestId(3),
            row: target,
            action: TreeBranchAction::Probe,
        },
    )
    .await;
    loop {
        match next_tree(&mut events).await {
            TreeEvent::Published(publication) => {
                assert_eq!(
                    publication.view.rows[0].entry.name, "z-large",
                    "rejected refresh must preserve frozen size order"
                );
                if publication
                    .view
                    .rows
                    .iter()
                    .any(|row| row.id == target && row.disclosure == TreeDisclosure::Empty)
                {
                    break;
                }
            }
            TreeEvent::Sizes(TreeSizeEvent::Snapshot { sizes }) => {
                tree(
                    &commands,
                    TreeCommand::Sizes(TreeSizeCommand::Acknowledge {
                        stamp: sizes.stamp,
                        version: sizes.version,
                    }),
                )
                .await;
                for row in &measured_rows {
                    assert!(
                        sizes.get(*row).is_some(),
                        "rejected refresh cannot erase known sizes"
                    );
                }
            }
            _ => {}
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_root_is_refused_then_commit_fences_selection_and_native_navigation() {
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(Files),
            send,
        )
        .run(receive),
    );
    let root = native_root(&commands, &mut events, 1).await;
    let mut foreign = root.clone();
    foreign.index = Arc::new((*root.index).clone());
    tree(
        &commands,
        TreeCommand::Enable {
            root: foreign,
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        },
    )
    .await;
    assert!(matches!(
        next_tree(&mut events).await,
        TreeEvent::Unavailable { .. }
    ));
    let publication = enable(&commands, &mut events, root).await;
    let row = publication.view.rows[0].id;
    let select = TreeSelectionRequest {
        view: publication.view.clone(),
        revision: SelectionRevision(1),
        seed: None,
        seed_revision: SelectionRevision(0),
        action: TreeSelectionAction::Toggle(row),
    };
    tree(&commands, TreeCommand::Select(select.clone())).await;
    assert!(matches!(
        next_tree(&mut events).await,
        TreeEvent::SelectionFailed { .. }
    ));
    tree(&commands, TreeCommand::Commit(publication.view.clone())).await;
    tree(&commands, TreeCommand::Select(select)).await;
    let TreeEvent::SelectionChanged {
        view,
        revision,
        selection,
    } = next_tree(&mut events).await
    else {
        panic!("selection expected")
    };
    assert_eq!(revision, SelectionRevision(1));
    assert!(Arc::ptr_eq(&view, &publication.view));
    assert!(selection.contains_row(row));
    tree(
        &commands,
        TreeCommand::Navigate {
            view,
            request: TreeRequestId(1),
            row,
            navigation: NavigationId(2),
        },
    )
    .await;
    let TreeEvent::Navigated {
        target: TreeNavigationTarget::Native(path),
        ..
    } = next_tree(&mut events).await
    else {
        panic!("typed native navigation expected")
    };
    assert_eq!(path, publication.view.rows[0].entry.path);
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[derive(Debug)]
struct Archive;
impl ArchiveMount for Archive {
    fn id(&self) -> u64 {
        77
    }
    fn list(&self, directory: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        Ok(vec![ArchiveNode {
            modified: None,
            id: VirtualEntryId {
                mount: 77,
                entry: directory + 1,
            },
            parent: directory,
            raw_name: Arc::from(&b"raw-\xff"[..]),
            display_name: "display only".into(),
            kind: EntryKind::Directory,
            size: None,
            readable: false,
        }])
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("no extraction")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_archive_completion_seeds_tree_and_navigation_retains_registration_after_pane_changes()
 {
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(Files),
            send,
        )
        .run(receive),
    );
    let location = ArchiveLocation {
        mount: Some(Arc::new(Archive)),
        directory: 1,
        address: ArchiveAddress {
            provider: "test.archive".into(),
            format: Some(ArchiveFormat::Zip),
            container: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/archive.zip"),
            source_version: vec![1],
            components: vec![],
        },
        trail: vec![(1, "archive.zip".into())],
    };
    commands
        .send(Command::ArchiveNavigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            action: ArchiveAction::List(location),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let root = loop {
        if let Event::ArchiveListed {
            pane,
            navigation,
            result,
        } = next(&mut events).await
        {
            let listing = result.unwrap();
            break TreeRootSnapshot {
                pane,
                navigation,
                entries: listing.entries,
                index: listing.index,
            };
        }
    };
    let publication = enable(&commands, &mut events, root).await;
    tree(&commands, TreeCommand::Commit(publication.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Navigate {
            view: publication.view.clone(),
            request: TreeRequestId(1),
            row: publication.view.rows[0].id,
            navigation: NavigationId(2),
        },
    )
    .await;
    let TreeEvent::Navigated {
        target: TreeNavigationTarget::Archive(target),
        ..
    } = next_tree(&mut events).await
    else {
        panic!("typed archive navigation expected")
    };
    assert_eq!(target.address.components, [b"raw-\xff".to_vec()]);
    assert_eq!(target.directory, 2);
    native_root(&commands, &mut events, 2).await;
    assert!(
        target.mount.as_ref().unwrap().validate().is_ok(),
        "navigation payload retains the registration across pane replacement"
    );
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
    assert!(target.mount.as_ref().unwrap().validate().is_ok());
}

#[derive(Default)]
struct Gate {
    released: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
    entered: tokio::sync::Notify,
    calls: std::sync::atomic::AtomicUsize,
    cancellation: std::sync::Mutex<Option<Cancellation>>,
}
impl Gate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
struct BlockingFiles(Arc<Gate>);
impl DirectoryService for BlockingFiles {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if path.file_name().is_some_and(|name| name == "東京 folder") {
            self.0
                .calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.0.cancellation.lock().unwrap() = Some(cancel.clone());
            self.0.entered.notify_one();
            let wait = self
                .0
                .changed
                .wait_timeout_while(
                    self.0.released.lock().unwrap(),
                    Duration::from_secs(5),
                    |released| !*released,
                )
                .unwrap();
            assert!(!wait.1.timed_out(), "test must release blocked provider");
            emit(Entry::new(
                EntryId(9),
                path.join("child"),
                EntryKind::Directory,
            ));
        } else {
            Files.read_directory(path, cancel, emit)?;
        }
        Ok(())
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("metadata must be reused")
    }
}
impl VolumeService for BlockingFiles {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for BlockingFiles {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        Files.space_info(path)
    }
}
async fn published(events: &mut mpsc::Receiver<Event>) -> TreePublication {
    let event = next_tree(events).await;
    let TreeEvent::Published(publication) = event else {
        panic!("publication expected: {event:?}")
    };
    publication
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_branch_can_be_collapsed_without_a_late_completion_reviving_children() {
    let gate = Arc::new(Gate::default());
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(BlockingFiles(gate.clone())),
            send,
        )
        .run(receive),
    );
    let root = native_root(&commands, &mut events, 1).await;
    let initial = enable(&commands, &mut events, root).await;
    tree(&commands, TreeCommand::Commit(initial.view.clone())).await;
    let row = initial.view.rows[0].id;
    tree(
        &commands,
        TreeCommand::Branch {
            view: initial.view,
            request: TreeRequestId(1),
            row,
            action: TreeBranchAction::Expand,
        },
    )
    .await;
    let loading = published(&mut events).await;
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    assert_eq!(loading.view.rows[0].disclosure, TreeDisclosure::Loading);
    tree(&commands, TreeCommand::Commit(loading.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Branch {
            view: loading.view,
            request: TreeRequestId(2),
            row,
            action: TreeBranchAction::Collapse,
        },
    )
    .await;
    let collapsed = published(&mut events).await;
    assert_eq!(collapsed.view.rows.len(), 2);
    assert!(
        gate.cancellation
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_cancelled()
    );
    gate.release();
    tree(&commands, TreeCommand::Commit(collapsed.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Select(TreeSelectionRequest {
            view: collapsed.view.clone(),
            revision: SelectionRevision(1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action: TreeSelectionAction::Toggle(row),
        }),
    )
    .await;
    let TreeEvent::SelectionChanged { view, .. } = next_tree(&mut events).await else {
        panic!("late completion must not replace collapsed view")
    };
    assert!(Arc::ptr_eq(&view, &collapsed.view));
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn navigation_preserves_branch_budget_until_the_old_provider_actually_returns() {
    let gate = Arc::new(Gate::default());
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(BlockingFiles(gate.clone())),
            send,
        )
        .run(receive),
    );
    let root = native_root(&commands, &mut events, 1).await;
    let initial = enable(&commands, &mut events, root).await;
    tree(&commands, TreeCommand::Commit(initial.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Branch {
            view: initial.view.clone(),
            request: TreeRequestId(1),
            row: initial.view.rows[0].id,
            action: TreeBranchAction::Expand,
        },
    )
    .await;
    published(&mut events).await;
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    let new_root = native_root(&commands, &mut events, 2).await;
    let newer = enable(&commands, &mut events, new_root).await;
    assert_ne!(initial.view.stamp.session, newer.view.stamp.session);
    tree(&commands, TreeCommand::Commit(newer.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Branch {
            view: newer.view.clone(),
            request: TreeRequestId(1),
            row: newer.view.rows[0].id,
            action: TreeBranchAction::Expand,
        },
    )
    .await;
    tree(
        &commands,
        TreeCommand::Select(TreeSelectionRequest {
            view: newer.view.clone(),
            revision: SelectionRevision(1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action: TreeSelectionAction::Bulk(SelectionOperation::Clear),
        }),
    )
    .await;
    assert!(matches!(
        next_tree(&mut events).await,
        TreeEvent::SelectionChanged { .. }
    ));
    assert_eq!(gate.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        gate.cancellation
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_cancelled()
    );
    gate.release();
    let loading = published(&mut events).await;
    let expanded = published(&mut events).await;
    assert_eq!(loading.view.stamp.navigation, NavigationId(2));
    assert_eq!(expanded.view.stamp.navigation, NavigationId(2));
    assert_eq!(expanded.view.rows.len(), 3);
    assert_eq!(gate.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[derive(Default)]
struct ValidationGate {
    enabled: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    released: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
}
impl ValidationGate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
#[derive(Debug)]
struct GatedArchive(Arc<ValidationGate>);
impl std::fmt::Debug for ValidationGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ValidationGate")
    }
}
impl ArchiveMount for GatedArchive {
    fn id(&self) -> u64 {
        Archive.id()
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        Archive.list(directory, cancel)
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("no extraction")
    }
    fn validate(&self) -> Result<(), FsError> {
        if self.0.enabled.load(std::sync::atomic::Ordering::SeqCst) {
            self.0.entered.notify_one();
            let wait = self
                .0
                .changed
                .wait_timeout_while(
                    self.0.released.lock().unwrap(),
                    Duration::from_secs(5),
                    |released| !*released,
                )
                .unwrap();
            assert!(!wait.1.timed_out(), "test must release validation callback");
        }
        Ok(())
    }
}
async fn archive_root(
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    mount: Arc<dyn ArchiveMount>,
) -> TreeRootSnapshot {
    archive_root_at(commands, events, mount, 1).await
}
async fn archive_root_at(
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    mount: Arc<dyn ArchiveMount>,
    generation: u64,
) -> TreeRootSnapshot {
    let location = ArchiveLocation {
        mount: Some(mount),
        directory: 1,
        address: ArchiveAddress {
            provider: "test.archive".into(),
            format: Some(ArchiveFormat::Zip),
            container: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/archive.zip"),
            source_version: vec![1],
            components: vec![],
        },
        trail: vec![(1, "archive.zip".into())],
    };
    commands
        .send(Command::ArchiveNavigate {
            pane: PaneId::Left,
            navigation: NavigationId(generation),
            action: ArchiveAction::List(location),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    loop {
        if let Event::ArchiveListed {
            pane,
            navigation,
            result,
        } = next(events).await
        {
            let listing = result.unwrap();
            break TreeRootSnapshot {
                pane,
                navigation,
                entries: listing.entries,
                index: listing.index,
            };
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_navigation_targets_share_one_pane_mount_budget_across_sessions() {
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(Files),
            send,
        )
        .run(receive),
    );
    let mut retained = Vec::new();
    for generation in 1..=64 {
        let root = archive_root_at(&commands, &mut events, Arc::new(Archive), generation).await;
        let publication = enable(&commands, &mut events, root).await;
        tree(&commands, TreeCommand::Commit(publication.view.clone())).await;
        tree(
            &commands,
            TreeCommand::Navigate {
                view: publication.view.clone(),
                request: TreeRequestId(1),
                row: publication.view.rows[0].id,
                navigation: NavigationId(generation + 1),
            },
        )
        .await;
        let TreeEvent::Navigated { target, .. } = next_tree(&mut events).await else {
            panic!("navigation target expected")
        };
        retained.push(target);
    }
    let root = archive_root_at(&commands, &mut events, Arc::new(Archive), 65).await;
    tree(
        &commands,
        TreeCommand::Enable {
            root: root.clone(),
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        },
    )
    .await;
    let limited = next_tree(&mut events).await;
    assert!(
        matches!(limited, TreeEvent::Unavailable { .. }),
        "retained targets must share a bounded registry: {limited:?}"
    );
    drop(retained);
    let publication = enable(&commands, &mut events, root).await;
    assert_eq!(publication.view.stamp.navigation, NavigationId(65));
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_root_payload_is_refused_before_it_enters_the_model_queue() {
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(Files),
            send,
        )
        .run(receive),
    );
    let root = native_root(&commands, &mut events, 1).await;
    let mut huge = root.clone();
    let mut entry = root.entries[0].clone();
    entry.display_name = "x".repeat(8 * 1024 * 1024 + 1);
    huge.entries = Arc::from([entry]);
    huge.index = Arc::new(DirectoryIndex::from_entries(&huge.entries));
    tree(
        &commands,
        TreeCommand::Enable {
            root: huge,
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        },
    )
    .await;
    let TreeEvent::Unavailable { error, .. } = next_tree(&mut events).await else {
        panic!("bounded admission expected")
    };
    assert!(
        error.as_str().contains("budget"),
        "byte screening precedes queued source admission: {error:?}"
    );
    enable(&commands, &mut events, root).await;
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[derive(Debug)]
struct NavigationBlockingArchive {
    listing: Arc<Gate>,
    validation: Arc<ValidationGate>,
}
struct CountingFiles(Arc<std::sync::atomic::AtomicUsize>);
impl DirectoryService for CountingFiles {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Files.read_directory(path, cancel, emit)
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("reuse rows")
    }
}
impl VolumeService for CountingFiles {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for CountingFiles {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        Files.space_info(path)
    }
}
impl std::fmt::Debug for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Gate")
    }
}
impl ArchiveMount for NavigationBlockingArchive {
    fn id(&self) -> u64 {
        77
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        if directory == 1 {
            Ok((2..=3)
                .map(|entry| ArchiveNode {
                    modified: None,
                    id: VirtualEntryId { mount: 77, entry },
                    parent: 1,
                    raw_name: Arc::from(format!("dir-{entry}").as_bytes()),
                    display_name: format!("dir-{entry}"),
                    kind: EntryKind::Directory,
                    size: None,
                    readable: false,
                })
                .collect())
        } else {
            *self.listing.cancellation.lock().unwrap() = Some(cancel.clone());
            self.listing.entered.notify_one();
            let wait = self
                .listing
                .changed
                .wait_timeout_while(
                    self.listing.released.lock().unwrap(),
                    Duration::from_secs(5),
                    |released| !*released,
                )
                .unwrap();
            assert!(!wait.1.timed_out());
            Ok(vec![])
        }
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("no extraction")
    }
    fn validate(&self) -> Result<(), FsError> {
        GatedArchive(self.validation.clone()).validate()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn navigation_cancels_branch_even_while_model_validation_is_blocked() {
    let listing = Arc::new(Gate::default());
    let validation = Arc::new(ValidationGate::default());
    let mount = Arc::new(NavigationBlockingArchive {
        listing: listing.clone(),
        validation: validation.clone(),
    });
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(CountingFiles(calls.clone())),
            send,
        )
        .run(receive),
    );
    let root = archive_root(&commands, &mut events, mount).await;
    let initial = enable(&commands, &mut events, root).await;
    tree(&commands, TreeCommand::Commit(initial.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Branch {
            view: initial.view.clone(),
            request: TreeRequestId(1),
            row: initial.view.rows[0].id,
            action: TreeBranchAction::Expand,
        },
    )
    .await;
    let loading = published(&mut events).await;
    tokio::time::timeout(Duration::from_secs(3), listing.entered.notified())
        .await
        .unwrap();
    tree(&commands, TreeCommand::Commit(loading.view.clone())).await;
    validation
        .enabled
        .store(true, std::sync::atomic::Ordering::SeqCst);
    tree(
        &commands,
        TreeCommand::Navigate {
            view: loading.view.clone(),
            request: TreeRequestId(2),
            row: loading.view.rows[1].id,
            navigation: NavigationId(2),
        },
    )
    .await;
    tokio::time::timeout(Duration::from_secs(3), validation.entered.notified())
        .await
        .unwrap();
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../_build/tree-controller-fixture"),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    loop {
        if matches!(
            next(&mut events).await,
            Event::ListingStarted {
                navigation: NavigationId(2),
                ..
            }
        ) {
            break;
        }
    }
    let cancelled = listing
        .cancellation
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .is_cancelled();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let native_calls = calls.load(std::sync::atomic::Ordering::SeqCst);
    validation.release();
    listing.release();
    assert!(
        cancelled,
        "out-of-band generation cancellation must reach the running provider before the model worker returns"
    );
    assert_eq!(
        native_calls, 0,
        "branch listing and model validation must own both common pane permits until actual return"
    );
    loop {
        if matches!(
            next(&mut events).await,
            Event::ListingFinished {
                navigation: NavigationId(2),
                ..
            }
        ) {
            break;
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_view_is_refused_before_it_can_queue_behind_provider_validation() {
    let gate = Arc::new(ValidationGate::default());
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(Files),
            send,
        )
        .run(receive),
    );
    let root = archive_root(&commands, &mut events, Arc::new(GatedArchive(gate.clone()))).await;
    let publication = enable(&commands, &mut events, root).await;
    tree(&commands, TreeCommand::Commit(publication.view.clone())).await;
    gate.enabled
        .store(true, std::sync::atomic::Ordering::SeqCst);
    tree(
        &commands,
        TreeCommand::Navigate {
            view: publication.view.clone(),
            request: TreeRequestId(1),
            row: publication.view.rows[0].id,
            navigation: NavigationId(2),
        },
    )
    .await;
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    tree(
        &commands,
        TreeCommand::Commit(Arc::new((*publication.view).clone())),
    )
    .await;
    let response = tokio::time::timeout(Duration::from_millis(200), events.recv()).await;
    gate.release();
    assert!(
        matches!(response, Ok(Some(Event::Tree(event))) if matches!(*event, TreeEvent::Rejected { .. })),
        "foreign view must never enter the model queue"
    );
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_root_index_is_refused_before_queueing() {
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(Files),
            send,
        )
        .run(receive),
    );
    let mut root = native_root(&commands, &mut events, 1).await;
    let mut index = (*root.index).clone();
    index.by_path.reserve(100_000);
    root.index = Arc::new(index);
    tree(
        &commands,
        TreeCommand::Enable {
            root,
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        },
    )
    .await;
    let TreeEvent::Unavailable { error, .. } = next_tree(&mut events).await else {
        panic!("budget refusal expected")
    };
    assert!(error.as_str().contains("budget"));
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewport_probes_only_directories_and_unsupported_file_requests_keep_the_session() {
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(Files),
            send,
        )
        .run(receive),
    );
    let root = native_root(&commands, &mut events, 1).await;
    let initial = enable(&commands, &mut events, root).await;
    let rows = TreeViewport::new(
        &initial.view,
        initial.view.rows.iter().map(|row| row.id).collect(),
    )
    .unwrap();
    tree(
        &commands,
        TreeCommand::Viewport {
            view: initial.view.clone(),
            rows: rows.clone(),
        },
    )
    .await;
    assert!(matches!(
        next_tree(&mut events).await,
        TreeEvent::Rejected { .. }
    ));
    tree(&commands, TreeCommand::Commit(initial.view.clone())).await;
    tree(
        &commands,
        TreeCommand::Viewport {
            view: initial.view,
            rows,
        },
    )
    .await;
    let loading = published(&mut events).await;
    assert_eq!(loading.view.rows[0].disclosure, TreeDisclosure::Loading);
    let probed = published(&mut events).await;
    assert_eq!(probed.view.rows.len(), 2);
    assert_eq!(probed.view.rows[0].disclosure, TreeDisclosure::Collapsed);
    assert!(!probed.view.rows[0].expanded);
    tree(&commands, TreeCommand::Commit(probed.view.clone())).await;
    let file = probed.view.rows[1].id;
    tree(
        &commands,
        TreeCommand::Navigate {
            view: probed.view.clone(),
            request: TreeRequestId(1),
            row: file,
            navigation: NavigationId(2),
        },
    )
    .await;
    assert!(matches!(
        next_tree(&mut events).await,
        TreeEvent::Rejected { .. }
    ));
    tree(
        &commands,
        TreeCommand::Branch {
            view: probed.view.clone(),
            request: TreeRequestId(2),
            row: file,
            action: TreeBranchAction::Expand,
        },
    )
    .await;
    assert!(matches!(
        next_tree(&mut events).await,
        TreeEvent::Rejected { .. }
    ));
    tree(
        &commands,
        TreeCommand::Select(TreeSelectionRequest {
            view: probed.view,
            revision: SelectionRevision(1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action: TreeSelectionAction::Toggle(file),
        }),
    )
    .await;
    assert!(matches!(
        next_tree(&mut events).await,
        TreeEvent::SelectionChanged { .. }
    ));
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_queue_rejects_saturation_and_reseeds_selection_before_commit() {
    let gate = Arc::new(ValidationGate::default());
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(Files),
            send,
        )
        .run(receive),
    );
    let root = archive_root(&commands, &mut events, Arc::new(GatedArchive(gate.clone()))).await;
    gate.enabled
        .store(true, std::sync::atomic::Ordering::SeqCst);
    tree(
        &commands,
        TreeCommand::Enable {
            root: root.clone(),
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        },
    )
    .await;
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    let selected =
        TreeRootSelection::Archive(Arc::new(std::collections::HashSet::from([EntryId(2)])));
    for revision in 1..=9 {
        tree(
            &commands,
            TreeCommand::Enable {
                root: root.clone(),
                selection_revision: SelectionRevision(revision),
                selected: selected.clone(),
            },
        )
        .await;
    }
    let TreeEvent::Unavailable { error, .. } = next_tree(&mut events).await else {
        panic!("queue saturation must be explicit")
    };
    assert!(error.as_str().contains("queue"));
    gate.release();
    let mut latest = published(&mut events).await;
    assert_eq!(latest.selection_revision, SelectionRevision(0));
    for revision in 1..=8 {
        latest = published(&mut events).await;
        assert_eq!(latest.selection_revision, SelectionRevision(revision));
    }
    assert!(latest.can_adopt(&root, SelectionRevision(8)));
    assert!(!latest.can_adopt(&root, SelectionRevision(9)));
    assert_eq!(latest.selection.len(), 1);
    assert!(latest.selection.contains_position(0));
    tree(&commands, TreeCommand::Commit(latest.view.clone())).await;
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnected_engine_releases_blocked_model_resources_after_the_callback_returns() {
    let gate = Arc::new(ValidationGate::default());
    let mount = Arc::new(GatedArchive(gate.clone()));
    let retained = Arc::downgrade(&mount);
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(Files),
            send,
        )
        .run(receive),
    );
    let root = archive_root(&commands, &mut events, mount).await;
    gate.enabled
        .store(true, std::sync::atomic::Ordering::SeqCst);
    tree(
        &commands,
        TreeCommand::Enable {
            root,
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        },
    )
    .await;
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    drop(commands);
    tokio::time::timeout(Duration::from_secs(2), engine)
        .await
        .unwrap()
        .unwrap();
    assert!(
        retained.upgrade().is_some(),
        "uncancellable callback still owns its backend"
    );
    gate.release();
    tokio::time::timeout(Duration::from_secs(2), async {
        while retained.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("model workers and queued roots must be released");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enable_reuses_the_actual_completed_root_and_publishes_selection_atomically() {
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(Files),
            send,
        )
        .run(receive),
    );
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../_build/tree-controller-fixture"),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let root = loop {
        if let Event::ListingFinished {
            pane,
            navigation,
            entries,
            index,
        } = next(&mut events).await
        {
            break TreeRootSnapshot {
                pane,
                navigation,
                entries,
                index,
            };
        }
    };
    commands
        .send(Command::Tree(Box::new(TreeCommand::Enable {
            root: root.clone(),
            selection_revision: SelectionRevision(7),
            selected: TreeRootSelection::Empty,
        })))
        .await
        .unwrap();
    let publication = loop {
        if let Event::Tree(event) = next(&mut events).await {
            let TreeEvent::Published(publication) = *event else {
                panic!("publication expected: {event:?}")
            };
            break publication;
        }
    };
    assert!(publication.view.root.same_snapshot(&root));
    assert_eq!(publication.view.rows.len(), 2);
    assert_eq!(publication.selection_revision, SelectionRevision(7));
    assert!(publication.selection.is_empty());
    assert!(Arc::ptr_eq(
        &publication.view,
        publication.selection.basis()
    ));
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}
