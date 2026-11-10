//! Persistent options through the real Engine, with observable provider I/O boundaries.
use midnight_core::*;
use midnight_engine::Engine;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Default)]
struct Files {
    lists: AtomicUsize,
    names: std::sync::Mutex<Option<Vec<String>>>,
}
impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        if let Some(names) = self.names.lock().unwrap().as_ref() {
            for (position, name) in names.iter().enumerate() {
                if !emit(Entry::new(
                    EntryId(position as u64 + 1),
                    path.join(name),
                    EntryKind::File,
                )) {
                    break;
                }
            }
            return Ok(());
        }
        let rows = if path.ends_with("Parent") {
            vec![
                (4, "child 東京", EntryKind::Directory),
                (5, "z-child", EntryKind::Directory),
            ]
        } else {
            vec![
                (1, "Parent", EntryKind::Directory),
                (2, "Other", EntryKind::Directory),
                (3, "file.txt", EntryKind::File),
            ]
        };
        for (id, name, kind) in rows {
            if !emit(Entry::new(EntryId(id), path.join(name), kind)) {
                break;
            }
        }
        Ok(())
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("metadata was complete")
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

#[derive(Debug, Default)]
struct Archive {
    lists: AtomicUsize,
    validates: AtomicUsize,
    gate: Gate,
}
impl ArchiveMount for Archive {
    fn id(&self) -> u64 {
        77
    }
    fn list(&self, directory: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        let rows = if directory == 1 {
            vec![(2, "Parent"), (3, "Other")]
        } else {
            vec![(4, "child 東京"), (5, "z-child")]
        };
        Ok(rows
            .into_iter()
            .map(|(id, caption)| ArchiveNode {
                id: VirtualEntryId {
                    mount: 77,
                    entry: id,
                },
                parent: directory,
                raw_name: if id == 2 {
                    Arc::from(&b"raw-parent-\xff"[..])
                } else {
                    Arc::from(caption.as_bytes())
                },
                display_name: caption.into(),
                kind: EntryKind::Directory,
                size: None,
                modified: None,
                readable: false,
            })
            .collect())
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("filtering cannot extract archive content")
    }
    fn validate(&self) -> Result<(), FsError> {
        self.validates.fetch_add(1, Ordering::SeqCst);
        self.gate.wait();
        Ok(())
    }
}

#[derive(Debug, Default)]
struct Gate {
    enabled: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    released: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
}
impl Gate {
    fn wait(&self) {
        if self.enabled.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            let outcome = self
                .changed
                .wait_timeout_while(
                    self.released.lock().unwrap(),
                    Duration::from_secs(5),
                    |released| !*released,
                )
                .unwrap();
            assert!(
                !outcome.1.timed_out(),
                "test must release provider validation"
            );
        }
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

struct Harness {
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
    engine: tokio::task::JoinHandle<()>,
    files: Arc<Files>,
    archive: Arc<Archive>,
    _directory: tempfile::TempDir,
}
impl Harness {
    fn new() -> Self {
        let (commands, receive) = mpsc::channel(32);
        let (send, events) = mpsc::channel(64);
        let files = Arc::new(Files::default());
        let engine = tokio::spawn(Engine::new(files.clone(), send).run(receive));
        Self {
            commands,
            events,
            engine,
            files,
            archive: Arc::default(),
            _directory: tempfile::tempdir().unwrap(),
        }
    }
    async fn event(&mut self) -> Event {
        tokio::time::timeout(Duration::from_secs(5), self.events.recv())
            .await
            .expect("event timeout")
            .expect("event stream")
    }
    async fn next(&mut self) -> TreeEvent {
        loop {
            if let Event::Tree(event) = self.event().await {
                return *event;
            }
        }
    }
    async fn send(&self, command: TreeCommand) {
        self.commands
            .send(Command::Tree(Box::new(command)))
            .await
            .unwrap();
    }
    async fn root(&mut self, archive: bool, navigation: u64) -> TreeRootSnapshot {
        let path = self._directory.path().to_owned();
        let command = if archive {
            Command::ArchiveNavigate {
                pane: PaneId::Left,
                navigation: NavigationId(navigation),
                sort: SortSpec::default(),
                action: ArchiveAction::List(ArchiveLocation {
                    mount: Some(self.archive.clone()),
                    directory: 1,
                    trail: vec![(1, "fixture.zip".into())],
                    address: ArchiveAddress {
                        provider: "test.archive".into(),
                        format: Some(ArchiveFormat::Zip),
                        container: path.join("fixture.zip"),
                        source_version: vec![1],
                        components: vec![],
                    },
                }),
            }
        } else {
            Command::Navigate {
                pane: PaneId::Left,
                navigation: NavigationId(navigation),
                path,
                sort: SortSpec::default(),
            }
        };
        self.commands.send(command).await.unwrap();
        loop {
            match self.event().await {
                Event::ListingFinished {
                    pane,
                    navigation,
                    entries,
                    index,
                } => {
                    return TreeRootSnapshot {
                        pane,
                        navigation,
                        entries,
                        index,
                    };
                }
                Event::ArchiveListed {
                    pane,
                    navigation,
                    result,
                } => {
                    let listing = result.unwrap();
                    return TreeRootSnapshot {
                        pane,
                        navigation,
                        entries: listing.entries,
                        index: listing.index,
                    };
                }
                _ => {}
            }
        }
    }
    async fn enable(&mut self, archive: bool) -> TreePublication {
        let root = self.root(archive, 1).await;
        self.send(TreeCommand::Enable {
            root,
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        })
        .await;
        let TreeEvent::Published(publication) = self.next().await else {
            panic!("initial tree expected")
        };
        self.send(TreeCommand::Commit(publication.view.clone()))
            .await;
        publication
    }
    async fn expanded(&mut self, archive: bool) -> TreePublication {
        let p = self.enable(archive).await;
        let row = p
            .view
            .rows
            .iter()
            .find(|r| r.entry.display_name == "Parent")
            .unwrap()
            .id;
        self.send(TreeCommand::Branch {
            view: p.view,
            request: TreeRequestId(1),
            row,
            action: TreeBranchAction::Expand,
        })
        .await;
        loop {
            let event = self.next().await;
            let TreeEvent::Published(p) = event else {
                panic!("expansion: {event:?}")
            };
            if p.view.rows.iter().any(|r| r.depth == 1) {
                self.send(TreeCommand::Commit(p.view.clone())).await;
                return p;
            }
        }
    }
    fn io_counts(&self) -> (usize, usize, usize) {
        (
            self.files.lists.load(Ordering::SeqCst),
            self.archive.lists.load(Ordering::SeqCst),
            self.archive.validates.load(Ordering::SeqCst),
        )
    }
    async fn block_model(&self, p: &TreePublication) {
        self.archive.gate.enabled.store(true, Ordering::SeqCst);
        self.send(TreeCommand::Navigate {
            view: p.view.clone(),
            request: TreeRequestId(1),
            row: p.view.rows[0].id,
            navigation: NavigationId(2),
        })
        .await;
        tokio::time::timeout(Duration::from_secs(3), self.archive.gate.entered.notified())
            .await
            .unwrap();
    }
    async fn finish(self) {
        self.commands.send(Command::Shutdown).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), self.engine)
            .await
            .unwrap()
            .unwrap();
    }
}
fn request(p: &TreePublication, id: u64, expression: &str) -> TreeReprojectRequest {
    TreeReprojectRequest {
        view: p.view.clone(),
        request: TreeRequestId(id),
        selection_revision: p.selection_revision,
        selection: p.selection.clone(),
        options: TreeProjectionOptions {
            search: Default::default(),
            sort: SortSpec::default(),
            filter: PanelFilter {
                enabled: !expression.is_empty(),
                mode: if expression.is_empty() {
                    FilterMode::Masks
                } else {
                    FilterMode::Expression
                },
                expression: expression.into(),
                case_sensitive: true,
            },
        },
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_filter_real_native_and_archive_preserve_roots_without_provider_io() {
    for archive in [false, true] {
        let mut h = Harness::new();
        let initial = h.expanded(archive).await;
        let counts = h.io_counts();
        let child = initial
            .view
            .rows
            .iter()
            .find(|r| r.entry.display_name == "child 東京")
            .unwrap()
            .clone();
        let req = request(&initial, 2, "name = \"child 東京\"");
        h.send(TreeCommand::Reproject(req.clone())).await;
        let event = h.next().await;
        let TreeEvent::Reprojected {
            previous,
            request: receipt,
            options,
            publication,
        } = event
        else {
            panic!("expected successful filter, got {event:?}")
        };
        assert_eq!(previous, initial.view.stamp);
        assert_eq!(receipt, req.request);
        assert_eq!(options, req.options);
        assert!(publication.view.root.same_snapshot(&initial.view.root));
        assert_eq!(
            publication
                .view
                .rows
                .iter()
                .map(|r| r.entry.display_name.as_str())
                .collect::<Vec<_>>(),
            ["Parent", "child 東京"]
        );
        assert!(publication.view.rows[0].context_only && !publication.view.rows[0].selectable);
        assert_eq!(publication.view.rows[1].id, child.id);
        let identity = |entry: &Entry| {
            entry
                .virtual_node
                .as_ref()
                .map(|node| (node.id, node.raw_name.clone()))
        };
        assert_eq!(
            identity(&publication.view.rows[1].entry),
            identity(&child.entry)
        );
        assert!(Arc::ptr_eq(
            &publication.view,
            publication.selection.basis()
        ));
        let mut clear = request(&publication, 3, "name = \"child 東京\"");
        clear.options.filter.enabled = false;
        clear.options.sort.descending = true;
        h.send(TreeCommand::Reproject(clear.clone())).await;
        assert!(
            matches!(
                h.next().await,
                TreeEvent::ReprojectFailed {
                    request: TreeRequestId(3),
                    ..
                }
            ),
            "new projection must require Commit"
        );
        h.send(TreeCommand::Commit(publication.view.clone())).await;
        h.send(TreeCommand::Reproject(clear.clone())).await;
        let event = h.next().await;
        let TreeEvent::Reprojected {
            publication: restored,
            options,
            ..
        } = event
        else {
            panic!("clear: {event:?}")
        };
        assert_eq!(options, clear.options);
        assert_eq!(restored.view.rows.len(), initial.view.rows.len());
        assert_eq!(restored.view.rows[0].entry.display_name, "Parent");
        assert_eq!(restored.view.rows[1].entry.display_name, "z-child");
        assert_eq!(restored.view.rows[2].entry.display_name, "child 東京");
        assert_eq!(
            h.io_counts(),
            counts,
            "filter/clear/sort performs no provider I/O"
        );
        h.send(TreeCommand::Commit(restored.view.clone())).await;
        h.send(TreeCommand::Reproject(clear)).await;
        assert!(matches!(h.next().await, TreeEvent::ReprojectFailed { .. }));
        h.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_filter_failed_requests_leave_the_original_view_usable() {
    let mut h = Harness::new();
    let p = h.enable(false).await;
    for case in 0..5 {
        let mut req = request(&p, case + 1, "kind = directory");
        match case {
            0 => req.options.filter.expression = "name =".into(),
            1 => req.selection = Arc::new((*p.selection).clone()),
            2 => req.view = Arc::new((*p.view).clone()),
            3 => req.selection_revision = SelectionRevision(10),
            4 => req.options.filter.expression = "x".repeat(4097),
            _ => unreachable!(),
        }
        h.send(TreeCommand::Reproject(req)).await;
        assert!(
            matches!(h.next().await, TreeEvent::ReprojectFailed { request, .. } if request == TreeRequestId(case + 1))
        );
    }
    h.send(TreeCommand::Reproject(request(&p, 6, "kind = directory")))
        .await;
    assert!(matches!(h.next().await, TreeEvent::Reprojected { .. }));
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_filter_queue_pressure_and_foreign_selection_are_rejected_before_provider_returns() {
    let mut h = Harness::new();
    let p = h.enable(true).await;
    h.block_model(&p).await;
    let mut foreign = request(&p, 2, "kind = directory");
    let mut view = (*p.view).clone();
    view.stamp.session = TreeSessionId(999);
    let view = Arc::new(view);
    foreign.selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    h.send(TreeCommand::Reproject(foreign)).await;
    let early = tokio::time::timeout(Duration::from_millis(500), h.next()).await;
    if early.is_err() {
        h.archive.gate.release();
    }
    assert!(
        matches!(
            early,
            Ok(TreeEvent::ReprojectFailed {
                request: TreeRequestId(2),
                ..
            })
        ),
        "foreign selection must not queue behind provider validation"
    );
    for id in 3..11 {
        h.send(TreeCommand::Reproject(request(&p, id, "kind = directory")))
            .await;
    }
    h.send(TreeCommand::Reproject(request(&p, 11, "kind = directory")))
        .await;
    let full = tokio::time::timeout(Duration::from_millis(500), h.next()).await;
    h.archive.gate.release();
    assert!(
        matches!(
            full,
            Ok(TreeEvent::ReprojectFailed {
                request: TreeRequestId(11),
                ..
            })
        ),
        "ninth queued command must fail promptly"
    );
    let mut successes = 0;
    let mut failures = 0;
    while successes + failures < 8 {
        match h.next().await {
            TreeEvent::Reprojected {
                request: TreeRequestId(3),
                ..
            } => successes += 1,
            TreeEvent::ReprojectFailed { .. } => failures += 1,
            TreeEvent::Navigated { .. } => {}
            event => panic!("unexpected queued result: {event:?}"),
        }
    }
    assert_eq!((successes, failures), (1, 7));
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_filter_cancelled_by_native_archive_network_navigation_or_shutdown() {
    for destination in 0..4 {
        let mut h = Harness::new();
        let p = h.enable(true).await;
        h.block_model(&p).await;
        h.send(TreeCommand::Reproject(request(&p, 2, "name = \"Other\"")))
            .await;
        if destination < 2 {
            h.root(destination == 1, 2).await;
        } else if destination == 2 {
            h.commands
                .send(Command::NetworkNavigate {
                    pane: PaneId::Left,
                    navigation: NavigationId(2),
                    action: NetworkAction::OpenRoot,
                    sort: SortSpec::default(),
                })
                .await
                .unwrap();
            loop {
                if matches!(
                    h.event().await,
                    Event::NetworkUpdated {
                        navigation: NavigationId(2),
                        ..
                    }
                ) {
                    break;
                }
            }
        } else {
            h.commands.send(Command::Shutdown).await.unwrap();
            tokio::time::timeout(Duration::from_secs(2), &mut h.engine)
                .await
                .unwrap()
                .unwrap();
            h.archive.gate.release();
            while let Some(event) = h.events.recv().await {
                assert!(
                    !matches!(event, Event::Tree(event) if matches!(*event, TreeEvent::Reprojected { .. }))
                );
            }
            continue;
        }
        h.archive.gate.release();
        // A new native root/session acts as a worker-order barrier after cancellation.
        let root = h.root(false, 3).await;
        h.send(TreeCommand::Enable {
            root,
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        })
        .await;
        loop {
            match h.next().await {
                TreeEvent::Published(next) => {
                    assert_eq!(next.view.stamp.navigation, NavigationId(3));
                    break;
                }
                TreeEvent::Reprojected { .. } => panic!("cancelled options were published"),
                _ => {}
            }
        }
        h.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_filter_valid_expression_with_excess_spare_capacity_is_normalized() {
    let mut h = Harness::new();
    let p = h.enable(false).await;
    let mut req = request(&p, 1, "kind = directory");
    req.options.filter.expression.reserve(1024 * 1024);
    h.send(TreeCommand::Reproject(req)).await;
    let event = h.next().await;
    let TreeEvent::Reprojected { options, .. } = event else {
        panic!("valid expression refused: {event:?}")
    };
    assert!(options.filter.expression.capacity() <= MAX_FILTER_BYTES);
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_filter_stale_selection_and_branch_requests_preserve_current_marks() {
    let mut h = Harness::new();
    let p = h.expanded(false).await;
    let child = p.view.rows.iter().find(|r| r.depth == 1).unwrap().id;
    h.send(TreeCommand::Select(TreeSelectionRequest {
        view: p.view.clone(),
        revision: SelectionRevision(1),
        seed: None,
        seed_revision: SelectionRevision(0),
        action: TreeSelectionAction::Toggle(child),
    }))
    .await;
    let TreeEvent::SelectionChanged {
        view,
        revision,
        selection,
    } = h.next().await
    else {
        panic!("selection acknowledgement expected")
    };
    assert!(selection.contains_row(child));
    h.send(TreeCommand::Reproject(request(&p, 2, "kind = file")))
        .await;
    assert!(matches!(h.next().await, TreeEvent::ReprojectFailed { .. }));
    let selected = TreePublication::new(view, revision, selection).unwrap();
    let parent = selected
        .view
        .rows
        .iter()
        .find(|r| r.entry.display_name == "Parent")
        .unwrap()
        .id;
    h.send(TreeCommand::Branch {
        view: selected.view.clone(),
        request: TreeRequestId(3),
        row: parent,
        action: TreeBranchAction::Collapse,
    })
    .await;
    h.send(TreeCommand::Reproject(request(&selected, 4, "kind = file")))
        .await;
    let mut collapsed = None;
    let mut rejected = false;
    while collapsed.is_none() || !rejected {
        match h.next().await {
            TreeEvent::Published(next) => collapsed = Some(next),
            TreeEvent::ReprojectFailed {
                request: TreeRequestId(4),
                ..
            } => rejected = true,
            other => panic!("collapse/reprojection interleaving: {other:?}"),
        }
    }
    let next = collapsed.unwrap();
    assert_eq!(next.view.rows.len(), 3);
    assert!(next.selection.is_empty());
    assert!(
        !next
            .view
            .rows
            .iter()
            .find(|r| r.id == parent)
            .unwrap()
            .expanded
    );
    h.send(TreeCommand::Commit(next.view.clone())).await;
    h.send(TreeCommand::Reproject(request(
        &next,
        5,
        "kind = directory",
    )))
    .await;
    assert!(matches!(h.next().await, TreeEvent::Reprojected { .. }));
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_filter_name_modes_keep_existing_folder_visibility_semantics() {
    let mut h = Harness::new();
    let mut p = h.enable(false).await;
    let counts = h.io_counts();
    for (id, mode, expression, expected) in [
        (1, FilterMode::Masks, "*.rs", 2),
        (2, FilterMode::Regex, r"\.txt$", 3),
        (3, FilterMode::Expression, "kind = file", 1),
    ] {
        let mut req = request(&p, id, expression);
        req.options.filter.mode = mode;
        h.send(TreeCommand::Reproject(req)).await;
        let event = h.next().await;
        let TreeEvent::Reprojected { publication, .. } = event else {
            panic!("mode result: {event:?}")
        };
        assert_eq!(publication.view.rows.len(), expected);
        h.send(TreeCommand::Commit(publication.view.clone())).await;
        p = publication;
    }
    assert_eq!(h.io_counts(), counts);
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_search_unicode_fold_graphemes_and_literal_names_use_backend_indexes() {
    let mut h = Harness::new();
    *h.files.names.lock().unwrap() = Some(vec![
        "Straße.txt".into(),
        "sStrange".into(),
        "a\u{301}bc".into(),
        "東京.txt".into(),
        "Σ.txt".into(),
        "👨‍👩‍👧‍👦.txt".into(),
        "[a].txt".into(),
    ]);
    let mut p = h.enable(false).await;
    let counts = h.io_counts();
    for (number, (query, expected, end)) in [
        ("STRASS", "Straße.txt", 6),
        ("a", "a\u{301}bc", 3),
        ("東", "東京.txt", 3),
        ("ς", "Σ.txt", 2),
        ("👨", "👨‍👩‍👧‍👦.txt", 25),
        ("[", "[a].txt", 1),
    ]
    .into_iter()
    .enumerate()
    {
        let mut req = request(&p, number as u64 + 1, "");
        req.options.search.query = query.into();
        h.send(TreeCommand::Reproject(req)).await;
        let event = h.next().await;
        let TreeEvent::Reprojected { publication, .. } = event else {
            panic!("search: {event:?}")
        };
        assert_eq!(publication.view.index.search_rows.len(), 1, "{query}");
        let matched = &publication.view.rows[publication.view.index.search_rows[0]];
        assert_eq!(matched.entry.display_name, expected);
        assert_eq!(matched.prefix_end, end);
        assert_eq!(publication.view.rows.len(), 7);
        h.send(TreeCommand::Commit(publication.view.clone())).await;
        p = publication;
    }
    assert_eq!(h.io_counts(), counts);
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_search_opaque_archive_caption_does_not_invent_a_raw_name_match() {
    let mut h = Harness::new();
    let mut p = h.expanded(true).await;
    let counts = h.io_counts();
    for (id, query, filter_matches, expected) in [(2, "PAR", false, 0), (3, "CHILD", true, 1)] {
        let mut req = request(&p, id, "");
        req.options.search.query = query.into();
        req.options.search.filter_matches = filter_matches;
        h.send(TreeCommand::Reproject(req)).await;
        let event = h.next().await;
        let TreeEvent::Reprojected { publication, .. } = event else {
            panic!("search: {event:?}")
        };
        assert_eq!(publication.view.index.search_rows.len(), expected);
        if filter_matches {
            assert_eq!(publication.view.rows.len(), 2);
            assert!(publication.view.rows[0].context_only);
            assert_eq!(
                &*publication.view.rows[0]
                    .entry
                    .virtual_node
                    .as_ref()
                    .unwrap()
                    .raw_name,
                b"raw-parent-\xff"
            );
            assert_eq!(publication.view.rows[1].prefix_end, 5);
            assert_eq!(&*publication.view.index.search_rows, &[1]);
        }
        h.send(TreeCommand::Commit(publication.view.clone())).await;
        p = publication;
    }
    assert_eq!(h.io_counts(), counts);
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_search_admission_rejects_invalid_queries_without_waiting_for_provider() {
    let mut h = Harness::new();
    let p = h.enable(true).await;
    h.block_model(&p).await;
    for (number, query) in ["x".repeat(4097), "bad\0query".into()]
        .into_iter()
        .enumerate()
    {
        let id = number as u64 + 2;
        let mut req = request(&p, id, "");
        req.options.search.query = query;
        h.send(TreeCommand::Reproject(req)).await;
        let early = tokio::time::timeout(Duration::from_millis(500), h.next()).await;
        if early.is_err() {
            h.archive.gate.release();
        }
        assert!(
            matches!(early, Ok(TreeEvent::ReprojectFailed { request, retryable: false, .. }) if request == TreeRequestId(id))
        );
    }
    h.archive.gate.release();
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_search_admission_normalizes_spare_query_capacity() {
    let mut h = Harness::new();
    let p = h.enable(false).await;
    let mut req = request(&p, 1, "");
    req.options.search.query = "file".into();
    req.options.search.query.reserve(1024 * 1024);
    h.send(TreeCommand::Reproject(req)).await;
    let event = h.next().await;
    let TreeEvent::Reprojected { options, .. } = event else {
        panic!("search: {event:?}")
    };
    assert!(options.search.query.capacity() <= 4096);
    h.finish().await;
}
