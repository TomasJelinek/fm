//! Headless production-controller boundaries with injected read-only native services.
use midnight_core::*;
use midnight_engine::Engine;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc as blocking,
    },
    time::{Duration, Instant},
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
    scaling: Option<Scaling>,
}

struct Active<'a>(&'a Files, bool, Option<Instant>);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        if self.1 {
            if let (Some(scaling), Some(started)) = (&self.0.scaling, self.2) {
                scaling.callback_ns.fetch_add(
                    u64::try_from(started.elapsed().as_nanos()).unwrap(),
                    Ordering::Relaxed,
                );
            }
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
            scaling: None,
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
        let object = if let Some(scaling) = &self.scaling {
            *scaling.indices.get(path).ok_or_else(|| {
                FsError::from_io(
                    "scaling identity",
                    path,
                    std::io::ErrorKind::NotFound.into(),
                )
            })? as u128
        } else if path == self.root {
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
        let _active = Active(
            self,
            branch,
            (branch && self.scaling.is_some()).then(Instant::now),
        );
        if branch {
            self.branch_calls.fetch_add(1, Ordering::AcqRel);
            let active = self.branch_active.fetch_add(1, Ordering::AcqRel) + 1;
            self.max_branch_active.fetch_max(active, Ordering::AcqRel);
        }
        if let Some(scaling) = &self.scaling {
            let index = *scaling.indices.get(path).ok_or_else(|| {
                FsError::from_io("scaling listing", path, std::io::ErrorKind::NotFound.into())
            })?;
            if branch {
                scaling.visits.lock().unwrap().push(index);
            }
            for child in &scaling.children[index] {
                if !emit(Entry::new(
                    EntryId(*child as u64),
                    scaling.paths[*child].clone(),
                    EntryKind::Directory,
                )) {
                    break;
                }
            }
            return Ok(());
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
    injected.wide = 256;
    let expected_calls = injected.wide + 1;
    let expected_rows = injected.wide + 2;
    assert!(expected_calls > 128, "the walk must exceed one full batch");
    let files = Arc::new(injected);
    let (commands, mut events, engine, initial) = start(files.clone()).await;
    recursive(&commands, &initial).await;
    let mut current = publication(&mut events).await;
    let first_calls = files.branch_calls.load(Ordering::Acquire);
    files.wait_finished(first_calls).await;
    assert!((1..=128).contains(&first_calls));
    assert!(
        first_calls < expected_calls,
        "unvisited work must remain when the first Commit is withheld"
    );
    assert!(
        current
            .view
            .rows
            .iter()
            .all(|row| row.disclosure != TreeDisclosure::Loading)
    );
    tree(&commands, TreeCommand::Commit(initial.view.clone())).await;
    let same_value = Arc::new((*current.view).clone());
    assert_eq!(same_value.stamp, current.view.stamp);
    assert!(!Arc::ptr_eq(&same_value, &current.view));
    tree(&commands, TreeCommand::Commit(same_value)).await;
    match next_tree(&mut events).await {
        TreeEvent::Rejected {
            stamp,
            request: TreeRequestId(0),
            row: TreeRowId(0),
            ..
        } => assert_eq!(stamp, current.view.stamp),
        other => panic!("same-value Arc cannot acknowledge the emitted view: {other:?}"),
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(80), next_tree(&mut events))
            .await
            .is_err()
    );
    assert_eq!(
        files.branch_calls.load(Ordering::Acquire),
        first_calls,
        "withheld exact Commit stops the next batch despite stale and same-value replies"
    );
    let mut publications = 0;
    loop {
        let done = current.view.rows.len() == expected_rows
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
        assert!(publications <= expected_calls);
        if done {
            break;
        }
        current = publication(&mut events).await;
    }
    assert_eq!(files.branch_calls.load(Ordering::Acquire), expected_calls);
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
    assert!(files.branch_calls.load(Ordering::Acquire) <= 128);
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

struct Scaling {
    paths: Vec<PathBuf>,
    indices: HashMap<PathBuf, usize>,
    children: Vec<Vec<usize>>,
    visits: Mutex<Vec<usize>>,
    callback_ns: AtomicU64,
}

impl Scaling {
    fn new(root: &Path, nodes: usize, trunk: usize) -> Self {
        assert!(nodes > trunk + 1);
        let mut paths = vec![root.to_owned()];
        let mut children = vec![Vec::new(); nodes];
        for level in 0..trunk {
            let name = if level == 0 {
                "folder".to_owned()
            } else {
                format!("d{level:02}")
            };
            paths.push(paths[level].join(name));
            children[level].push(level + 1);
        }
        for index in 0..nodes - trunk - 1 {
            // Gray-code names are short, unique and not emitted in sorted order.
            let name = format!("n{:04x}", index ^ (index >> 1));
            children[trunk].push(paths.len());
            paths.push(paths[trunk].join(name));
        }
        let indices = paths
            .iter()
            .enumerate()
            .map(|(index, path)| (path.clone(), index))
            .collect();
        Self {
            paths,
            indices,
            children,
            visits: Mutex::new(Vec::with_capacity(nodes)),
            callback_ns: AtomicU64::new(0),
        }
    }
}

async fn scaling_event(events: &mut mpsc::Receiver<Event>) -> TreeEvent {
    loop {
        match events.recv().await {
            Some(Event::Tree(event)) => return *event,
            Some(_) => {}
            None => panic!("scaling event stream closed"),
        }
    }
}

async fn scaling_ack_size(commands: &mpsc::Sender<Command>, sizes: &TreeSizeSnapshot) {
    tree(
        commands,
        TreeCommand::Sizes(TreeSizeCommand::Acknowledge {
            stamp: sizes.stamp,
            version: sizes.version,
        }),
    )
    .await;
}

async fn scaling_setup_publication(
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
) -> TreePublication {
    loop {
        match scaling_event(events).await {
            TreeEvent::Published(publication) => {
                tree(commands, TreeCommand::Commit(publication.view.clone())).await;
                assert!(publication.view.rows.iter().all(|row| row.error.is_none()));
                return publication;
            }
            TreeEvent::Sizes(TreeSizeEvent::Snapshot { sizes }) => {
                scaling_ack_size(commands, &sizes).await;
            }
            TreeEvent::Sizes(TreeSizeEvent::Selected { .. }) => {}
            other => panic!("scaling setup failed: {other:?}"),
        }
    }
}

async fn scaling_seed_size(
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    publication: &TreePublication,
    request: TreeRequestId,
) {
    let leaf = publication.view.rows.last().unwrap().id;
    tree(
        commands,
        TreeCommand::Sizes(TreeSizeCommand::Compute(TreeSizeRequest {
            view: publication.view.clone(),
            request,
            selection_revision: publication.selection_revision,
            selection: publication.selection.clone(),
            scope: TreeSizeScope::Single(leaf),
        })),
    )
    .await;
    let (mut finished, mut settled) = (false, false);
    while !finished || !settled {
        match scaling_event(events).await {
            TreeEvent::Sizes(TreeSizeEvent::Snapshot { sizes }) => {
                scaling_ack_size(commands, &sizes).await;
                if !sizes.running(leaf)
                    && (finished || sizes.get(leaf).is_some_and(|size| size.finished))
                {
                    assert_eq!(sizes.len(), 1, "scaling seed final value count");
                    let size = sizes.get(leaf).expect("scaling seed final leaf value");
                    assert!(
                        size.finished && size.errors == 0 && size.bytes == 0,
                        "invalid scaling seed final value: {size:?}"
                    );
                    settled = true;
                }
            }
            TreeEvent::Sizes(TreeSizeEvent::Finished {
                request: actual,
                cancelled: false,
                ..
            }) if actual == request => finished = true,
            TreeEvent::Sizes(TreeSizeEvent::Selected { .. }) => {}
            other => panic!("scaling size seed failed: {other:?}"),
        }
    }
}

#[derive(Debug, Default)]
struct ScalingMetrics {
    batches: Vec<usize>,
    loading_rows: usize,
    errors: usize,
    max_receive_to_commit_ns: u128,
    receive_to_commit_ns: u128,
    failure: Option<String>,
}

async fn scaling_setup(
    files: Arc<Files>,
    nodes: usize,
    trunk: usize,
    sizes: usize,
) -> (
    mpsc::Sender<Command>,
    mpsc::Receiver<Event>,
    tokio::task::JoinHandle<()>,
    TreePublication,
) {
    let scaling = files.scaling.as_ref().unwrap();
    let (commands, mut events, engine, mut current) = start(files.clone()).await;
    tree(&commands, TreeCommand::Commit(current.view.clone())).await;
    for level in 1..=trunk {
        let row = current
            .view
            .rows
            .iter()
            .find(|row| row.entry.native_path() == Some(scaling.paths[level].as_path()))
            .unwrap()
            .id;
        tree(
            &commands,
            TreeCommand::Branch {
                view: current.view.clone(),
                request: TreeRequestId(level as u64),
                row,
                action: TreeBranchAction::Expand,
            },
        )
        .await;
        loop {
            current = scaling_setup_publication(&commands, &mut events).await;
            if current
                .view
                .rows
                .iter()
                .any(|item| item.id == row && item.disclosure == TreeDisclosure::Expanded)
            {
                break;
            }
        }
    }
    assert_eq!(current.view.rows.len(), nodes - 1, "setup admission");
    if sizes == 1 {
        scaling_seed_size(
            &commands,
            &mut events,
            &current,
            TreeRequestId(trunk as u64 + 1),
        )
        .await;
    }
    (commands, events, engine, current)
}

async fn scaling_case(nodes: usize, trunk: usize, sizes: usize, measured: bool) {
    eprintln!("recursive_scaling phase=setup N={nodes} trunk={trunk} K={sizes} timed={measured}");
    let mut injected = Files::new();
    injected.scaling = Some(Scaling::new(&injected.root, nodes, trunk));
    let files = Arc::new(injected);
    let scaling = files.scaling.as_ref().unwrap();
    let (commands, mut events, engine, mut current) = tokio::time::timeout(
        Duration::from_secs(60),
        scaling_setup(files.clone(), nodes, trunk, sizes),
    )
    .await
    .expect("scaling setup exceeded its separate 60 s deadline");
    assert_eq!(files.branch_active.load(Ordering::Acquire), 0);
    files.branch_calls.store(0, Ordering::Release);
    files.branch_finished.store(0, Ordering::Release);
    files.max_branch_active.store(0, Ordering::Release);
    scaling.visits.lock().unwrap().clear();
    scaling.callback_ns.store(0, Ordering::Relaxed);
    let visible = current.view.rows.len();
    let depth = current.view.rows.iter().map(|row| row.depth).max().unwrap();
    let root = current.view.rows[0].id;
    let mut metrics = ScalingMetrics::default();
    let mut returned = 0;
    // Every admitted node is visible here, so retained projection rows equal V.
    eprintln!(
        "recursive_scaling phase=run timed={measured} N={} R_derived={visible} \
         V={visible} depth={depth} K={sizes}",
        visible + 1
    );
    let started = Instant::now();
    let mut progress = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(10),
        Duration::from_secs(10),
    );
    let result = tokio::time::timeout(Duration::from_secs(60), async {
        tree(
            &commands,
            TreeCommand::Branch {
                view: current.view.clone(),
                request: TreeRequestId(trunk as u64 + 2),
                row: root,
                action: TreeBranchAction::ExpandRecursive,
            },
        )
        .await;
        loop {
            let event = tokio::select! {
                event = scaling_event(&mut events) => event,
                _ = progress.tick() => {
                    eprintln!(
                        "recursive_scaling phase=progress timed={measured} F={returned} \
                         B={} callbacks={} \
                         callback_ns={} max_active={} wall_ns={} errors={} loading_rows={}",
                        metrics.batches.len(),
                        files.branch_calls.load(Ordering::Acquire),
                        scaling.callback_ns.load(Ordering::Relaxed),
                        files.max_branch_active.load(Ordering::Acquire),
                        started.elapsed().as_nanos(),
                        metrics.errors,
                        metrics.loading_rows,
                    );
                    continue;
                }
            };
            match event {
                TreeEvent::Published(publication) => {
                    let received = Instant::now();
                    let finished = files.branch_finished.load(Ordering::Acquire);
                    tree(&commands, TreeCommand::Commit(publication.view.clone())).await;
                    let delay = received.elapsed().as_nanos();
                    metrics.receive_to_commit_ns += delay;
                    metrics.max_receive_to_commit_ns = metrics.max_receive_to_commit_ns.max(delay);
                    metrics.batches.push(finished - returned);
                    returned = finished;
                    for row in &*publication.view.rows {
                        metrics.loading_rows +=
                            usize::from(row.disclosure == TreeDisclosure::Loading);
                        metrics.errors += usize::from(row.error.is_some());
                    }
                    current = publication;
                    if returned >= nodes - 1 || metrics.errors != 0 || metrics.loading_rows != 0 {
                        break;
                    }
                }
                TreeEvent::Sizes(TreeSizeEvent::Snapshot { sizes: snapshot }) => {
                    scaling_ack_size(&commands, &snapshot).await;
                    if snapshot.len() != sizes {
                        metrics.failure = Some("settled size count changed".into());
                        break;
                    }
                }
                TreeEvent::Sizes(TreeSizeEvent::Selected { .. }) => {}
                other => {
                    metrics.failure = Some(format!("recursive event: {other:?}"));
                    break;
                }
            }
        }
    })
    .await;
    let elapsed = started.elapsed();
    if result.is_err() {
        metrics.failure = Some("censored at the 60 s timed deadline".into());
    }
    eprintln!(
        "recursive_scaling timed={measured} N={} R_derived={visible} V={} F={returned} K={sizes} \
         B_publications={} callbacks={} callback_ns={} max_active={} wall_ns={} \
         metrics={metrics:?}",
        visible + 1,
        current.view.rows.len(),
        metrics.batches.len(),
        files.branch_calls.load(Ordering::Acquire),
        scaling.callback_ns.load(Ordering::Relaxed),
        files.max_branch_active.load(Ordering::Acquire),
        elapsed.as_nanos(),
    );
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
    assert!(metrics.failure.is_none(), "{:?}", metrics.failure);
    assert_eq!(metrics.loading_rows, 0);
    assert_eq!(metrics.errors, 0);
    assert!(
        metrics
            .batches
            .iter()
            .all(|count| (1..=128).contains(count))
    );
    assert_eq!(returned, nodes - 1);
    assert_eq!(files.branch_calls.load(Ordering::Acquire), nodes - 1);
    assert_eq!(files.branch_finished.load(Ordering::Acquire), nodes - 1);
    assert_eq!(files.max_branch_active.load(Ordering::Acquire), 1);
    assert_eq!(current.view.rows.len(), nodes - 1);
    assert!(current.view.rows.iter().all(|row| matches!(
        row.disclosure,
        TreeDisclosure::Empty | TreeDisclosure::Expanded
    )));
    let mut visits = scaling.visits.lock().unwrap().clone();
    visits.sort_unstable();
    assert_eq!(visits, (1..nodes).collect::<Vec<_>>());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "opt-in serial before/after scaling control; externally cap each case at 60 s"]
async fn recursive_scaling_before_after() {
    scaling_case(32, 1, 0, false).await;
    scaling_case(1_000, 1, 0, true).await;
    scaling_case(10_000, 1, 0, true).await;
    scaling_case(10_000, 1, 1, true).await;
    scaling_case(10_000, 32, 0, true).await;
}
