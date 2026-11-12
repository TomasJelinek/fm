//! Headless production-controller boundaries with injected read-only native services.
use midnight_core::*;
use midnight_engine::Engine;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc as blocking,
    },
    time::Duration,
};
use tokio::sync::{Notify, mpsc, oneshot};

struct Block {
    path: PathBuf,
    entered: oneshot::Sender<()>,
    release: blocking::Receiver<()>,
}

struct Files {
    root: PathBuf,
    branch_calls: AtomicUsize,
    branch_finished: AtomicUsize,
    branch_active: AtomicUsize,
    max_branch_active: AtomicUsize,
    completed: Notify,
    block: Mutex<Option<Block>>,
    wide: usize,
}

struct Active<'a>(&'a Files, bool);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        if self.1 {
            self.0.branch_active.fetch_sub(1, Ordering::AcqRel);
            self.0.branch_finished.fetch_add(1, Ordering::Release);
            self.0.completed.notify_one();
        }
    }
}

impl Files {
    fn new() -> Self {
        Self {
            root: std::env::temp_dir().join("midnight-recursive-controller-injected"),
            branch_calls: AtomicUsize::new(0),
            branch_finished: AtomicUsize::new(0),
            branch_active: AtomicUsize::new(0),
            max_branch_active: AtomicUsize::new(0),
            completed: Notify::new(),
            block: Mutex::default(),
            wide: 0,
        }
    }

    async fn wait_finished(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let notified = self.completed.notified();
                if self.branch_finished.load(Ordering::Acquire) >= count {
                    break;
                }
                notified.await;
            }
        })
        .await
        .unwrap();
    }
}

impl DirectoryService for Files {
    fn resolve_size_directory(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Option<SizeDirectoryTarget>, FsError> {
        let object = if path == self.root {
            0_u128
        } else if path == self.root.join("folder") {
            1
        } else if path == self.root.join("folder/child-one") {
            2
        } else if path == self.root.join("folder/child-two") {
            3
        } else if path == self.root.join("folder/child-one/grandchild") {
            4
        } else {
            5
        };
        Ok(Some(SizeDirectoryTarget {
            path: path.to_owned(),
            identity: SizeDirectoryIdentity {
                device: 1,
                object: object.to_le_bytes(),
            },
        }))
    }

    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        let branch = path.starts_with(self.root.join("folder"));
        let _active = Active(self, branch);
        if branch {
            self.branch_calls.fetch_add(1, Ordering::AcqRel);
            let active = self.branch_active.fetch_add(1, Ordering::AcqRel) + 1;
            self.max_branch_active.fetch_max(active, Ordering::AcqRel);
        }
        let blocked = {
            let mut block = self.block.lock().unwrap();
            if block.as_ref().is_some_and(|block| block.path == path) {
                block.take()
            } else {
                None
            }
        };
        if let Some(block) = blocked {
            let _ = block.entered.send(());
            block.release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        if self.wide != 0 && path == self.root.join("folder") {
            for index in 0..self.wide {
                if !emit(Entry::new(
                    EntryId(index as u64 + 1),
                    path.join(format!("child-{index}")),
                    EntryKind::Directory,
                )) {
                    break;
                }
            }
            return Ok(());
        }
        let rows = if path == self.root {
            vec![
                ("folder", EntryKind::DirectoryLink),
                ("other", EntryKind::Directory),
            ]
        } else if path == self.root.join("folder") {
            vec![
                ("child-one", EntryKind::Directory),
                ("child-two", EntryKind::Directory),
            ]
        } else if path == self.root.join("folder/child-one") {
            vec![("grandchild", EntryKind::Directory)]
        } else {
            Vec::new()
        };
        for (index, (name, kind)) in rows.into_iter().enumerate() {
            if !emit(Entry::new(EntryId(index as u64 + 1), path.join(name), kind)) {
                break;
            }
        }
        Ok(())
    }

    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        Ok(EntryMetadata::default())
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}
impl StorageService for Files {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            available: 1,
            total: 1,
        })
    }
}

async fn tree(commands: &mpsc::Sender<Command>, command: TreeCommand) {
    commands
        .send(Command::Tree(Box::new(command)))
        .await
        .unwrap();
}
async fn next_tree(events: &mut mpsc::Receiver<Event>) -> TreeEvent {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Event::Tree(event)) = events.recv().await {
                break *event;
            }
        }
    })
    .await
    .unwrap()
}
async fn publication(events: &mut mpsc::Receiver<Event>) -> TreePublication {
    match next_tree(events).await {
        TreeEvent::Published(publication) => publication,
        other => panic!("expected publication: {other:?}"),
    }
}

async fn listing(events: &mut mpsc::Receiver<Event>, navigation: NavigationId) -> TreeRootSnapshot {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Event::ListingFinished {
                pane,
                navigation: actual,
                entries,
                index,
            }) = events.recv().await
                && actual == navigation
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
    .unwrap()
}

async fn start(
    files: Arc<Files>,
) -> (
    mpsc::Sender<Command>,
    mpsc::Receiver<Event>,
    tokio::task::JoinHandle<()>,
    TreePublication,
) {
    let (commands, receive) = mpsc::channel(16);
    let (send, mut events) = mpsc::channel(32);
    let engine =
        tokio::spawn(Engine::new(Engine::lifetime().unwrap(), files.clone(), send).run(receive));
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: files.root.clone(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let root = listing(&mut events, NavigationId(1)).await;
    tree(
        &commands,
        TreeCommand::Enable {
            root,
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        },
    )
    .await;
    let publication = publication(&mut events).await;
    (commands, events, engine, publication)
}

async fn recursive(commands: &mpsc::Sender<Command>, publication: &TreePublication) {
    tree(commands, TreeCommand::Commit(publication.view.clone())).await;
    let row = publication
        .view
        .rows
        .iter()
        .find(|row| row.entry.display_name == "folder")
        .unwrap()
        .id;
    tree(
        commands,
        TreeCommand::Branch {
            view: publication.view.clone(),
            request: TreeRequestId(1),
            row,
            action: TreeBranchAction::ExpandRecursive,
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recursive_controller_reuses_one_branch_worker_and_waits_for_publication_commit() {
    let mut injected = Files::new();
    injected.wide = 48;
    let files = Arc::new(injected);
    let (commands, mut events, engine, initial) = start(files.clone()).await;
    recursive(&commands, &initial).await;
    let mut current = publication(&mut events).await;
    let first_calls = files.branch_calls.load(Ordering::Acquire);
    files.wait_finished(first_calls).await;
    assert!((1..=16).contains(&first_calls));
    assert!(
        current
            .view
            .rows
            .iter()
            .all(|row| row.disclosure != TreeDisclosure::Loading)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(80), next_tree(&mut events))
            .await
            .is_err()
    );
    assert_eq!(
        files.branch_calls.load(Ordering::Acquire),
        first_calls,
        "withheld Commit stops the next batch"
    );
    let mut publications = 0;
    loop {
        let done = current.view.rows.len() == 50
            && current
                .view
                .rows
                .iter()
                .filter(|row| {
                    row.entry
                        .native_path()
                        .is_some_and(|path| path.starts_with(files.root.join("folder")))
                })
                .all(|row| {
                    matches!(
                        row.disclosure,
                        TreeDisclosure::Empty | TreeDisclosure::Expanded
                    )
                });
        tree(&commands, TreeCommand::Commit(current.view)).await;
        publications += 1;
        assert!(publications <= 49);
        if done {
            break;
        }
        current = publication(&mut events).await;
    }
    assert_eq!(files.branch_calls.load(Ordering::Acquire), 49);
    assert_eq!(files.max_branch_active.load(Ordering::Acquire), 1);
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn navigation_cancels_a_blocked_recursive_descendant_and_late_results_do_not_reopen() {
    let files = Arc::new(Files::new());
    let (entered, mut entered_rx) = oneshot::channel();
    let (release, release_rx) = blocking::channel();
    *files.block.lock().unwrap() = Some(Block {
        path: files.root.join("folder/child-one"),
        entered,
        release: release_rx,
    });
    let (commands, mut events, engine, initial) = start(files.clone()).await;
    recursive(&commands, &initial).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            tokio::select! {
                entered = &mut entered_rx => { entered.unwrap(); break; }
                current = publication(&mut events) => tree(&commands, TreeCommand::Commit(current.view)).await,
            }
        }
    }).await.unwrap();
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            path: files.root.join("replacement"),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if matches!(
                events.recv().await,
                Some(Event::ListingStarted {
                    navigation: NavigationId(2),
                    ..
                })
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();
    release.send(()).unwrap();
    let root = listing(&mut events, NavigationId(2)).await;
    tree(
        &commands,
        TreeCommand::Enable {
            root,
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        },
    )
    .await;
    let replacement = loop {
        let current = publication(&mut events).await;
        if current.view.stamp.navigation == NavigationId(2) {
            break current;
        }
    };
    assert!(replacement.view.rows.is_empty());
    tree(&commands, TreeCommand::Commit(replacement.view)).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(80), next_tree(&mut events))
            .await
            .is_err()
    );
    assert_eq!(
        files.branch_calls.load(Ordering::Acquire),
        2,
        "no recursive siblings start after navigation"
    );
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_projection_capacity_reports_against_the_emitted_view_and_keeps_navigation_available()
{
    let mut injected = Files::new();
    for _ in 0..185 {
        injected.root.push("valid-segment-1234");
    }
    assert!(injected.root.as_os_str().len() < 3800);
    injected.wide = 2500;
    let files = Arc::new(injected);
    let (commands, mut events, engine, initial) = start(files.clone()).await;
    recursive(&commands, &initial).await;
    match next_tree(&mut events).await {
        TreeEvent::Rejected {
            stamp,
            request: TreeRequestId(1),
            error,
            ..
        } => {
            assert_eq!(
                stamp, initial.view.stamp,
                "capacity preserves the last published view"
            );
            assert!(error.as_str().contains("incomplete"));
        }
        other => panic!("expected explicit incomplete status: {other:?}"),
    }
    let row = initial
        .view
        .rows
        .iter()
        .find(|row| row.entry.display_name == "other")
        .unwrap()
        .id;
    tree(
        &commands,
        TreeCommand::Navigate {
            view: initial.view,
            request: TreeRequestId(2),
            row,
            navigation: NavigationId(2),
        },
    )
    .await;
    assert!(
        matches!(next_tree(&mut events).await, TreeEvent::Navigated { target: TreeNavigationTarget::Native(path), .. } if path == files.root.join("other"))
    );
    assert!(files.branch_calls.load(Ordering::Acquire) <= 16);
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_collapse_rolls_back_staged_rows_and_keeps_a_blocked_native_permit_owned() {
    let files = Arc::new(Files::new());
    let (entered, mut entered_rx) = oneshot::channel();
    let (release, release_rx) = blocking::channel();
    *files.block.lock().unwrap() = Some(Block {
        path: files.root.join("folder/child-one"),
        entered,
        release: release_rx,
    });
    let (commands, mut events, engine, mut visible) = start(files.clone()).await;
    recursive(&commands, &visible).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            tokio::select! {
                entered = &mut entered_rx => { entered.unwrap(); break; }
                current = publication(&mut events) => {
                    tree(&commands, TreeCommand::Commit(current.view.clone())).await;
                    visible = current;
                }
            }
        }
    })
    .await
    .unwrap();
    let row = visible
        .view
        .rows
        .iter()
        .find(|row| row.entry.display_name == "folder")
        .unwrap()
        .id;
    tree(
        &commands,
        TreeCommand::Branch {
            view: visible.view,
            request: TreeRequestId(2),
            row,
            action: TreeBranchAction::Collapse,
        },
    )
    .await;
    let collapsed = publication(&mut events).await;
    assert!(collapsed.view.rows.iter().all(|row| row.depth == 0));
    assert!(
        !collapsed
            .view
            .rows
            .iter()
            .find(|item| item.id == row)
            .unwrap()
            .expanded
    );
    assert_eq!(
        files.branch_active.load(Ordering::Acquire),
        1,
        "cancellation cannot invent callback return"
    );
    release.send(()).unwrap();
    files.wait_finished(2).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(80), next_tree(&mut events))
            .await
            .is_err()
    );
    assert_eq!(files.branch_calls.load(Ordering::Acquire), 2);
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}
