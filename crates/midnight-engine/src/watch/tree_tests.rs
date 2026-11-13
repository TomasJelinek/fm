use super::*;
use crate::{
    providers::NativeProvider,
    tree_actor::{Session, WorkerBudget},
    tree_projection::ProjectionOptions,
};
use midnight_core::{TreeEvent, TreeRequestId, TreeRootSnapshot};

fn scope(files: &Arc<Files>, path: &str) -> (Session, tree::TreeScope) {
    let visible = files.visible(PaneId::Left, 4, path);
    scope_from_visible(files.clone(), visible)
}

fn scope_from_visible(
    files: Arc<dyn midnight_core::FileSystem>,
    visible: VisibleDirectory,
) -> (Session, tree::TreeScope) {
    let root = TreeRootSnapshot {
        pane: visible.pane,
        navigation: visible.navigation,
        index: Arc::new(midnight_core::DirectoryIndex::from_entries(
            &visible.entries,
        )),
        entries: visible.entries,
    };
    let native = NativeProvider::new(files);
    let cancel = Cancellation::default();
    let opened = native
        .capture_root(&visible.path, &root.entries, &cancel)
        .unwrap();
    let mut session = Session::new(
        root,
        opened,
        Arc::default(),
        WorkerBudget::new(Arc::new(tokio::sync::Semaphore::new(1))),
        ProjectionOptions::default(),
        &cancel,
        crate::ReportArena::testing(),
    )
    .unwrap();
    let view = session.projection().clone();
    assert!(session.commit(&view));
    let scope = session.watch_scope().unwrap().unwrap();
    (session, scope)
}

async fn tree_changed(events: &mut mpsc::Receiver<Event>) -> midnight_core::TreeViewStamp {
    match tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Event::Tree(event) => match *event {
            TreeEvent::Changed { stamp } => stamp,
            other => panic!("unexpected tree event {other:?}"),
        },
        other => panic!("unexpected watch event {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_navigation_success_then_exact_commit_quiets_watch() {
    let files = Arc::new(Files::default());
    let path = std::env::current_dir()
        .unwrap()
        .join("_build/watch-tree")
        .to_str()
        .unwrap()
        .to_owned();
    files.put(&path, 7);
    let (_session, scope) = scope(&files, &path);
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(70));
    let handle = controller.handle();
    handle.set_tree_scope(scope.clone());
    let stamp = tree_changed(&mut events).await;
    let ticket = handle.begin_refresh(stamp, TreeRequestId(1)).unwrap();
    handle.finish_refresh(ticket, true, stamp.revision);
    handle.set_tree_scope(scope);
    // Acceptance cannot retract hints already queued before its fence.
    while events.try_recv().is_ok() {}
    assert!(
        tokio::time::timeout(Duration::from_millis(240), events.recv())
            .await
            .is_err()
    );
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_only_poll_between_sweep_and_commit_survives() {
    let files = Arc::new(Files::default());
    let path = std::env::current_dir()
        .unwrap()
        .join("_build/watch-tree-late")
        .to_str()
        .unwrap()
        .to_owned();
    files.put(&path, 7);
    let (_session, scope) = scope(&files, &path);
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(70));
    let handle = controller.handle();
    handle.set_tree_scope(scope.clone());
    let stamp = tree_changed(&mut events).await;
    let ticket = handle.begin_refresh(stamp, TreeRequestId(1)).unwrap();
    handle.finish_refresh(ticket, true, stamp.revision);
    files.put(&path, 8);
    files.wait_probes(2).await;
    handle.set_tree_scope(scope);
    assert_eq!(tree_changed(&mut events).await.session, stamp.session);
    handle.clear_scope(PaneId::Left, NavigationId(5));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_flat_ack_cannot_replace_adopted_scope() {
    let files = Arc::new(Files::default());
    let path = std::env::current_dir()
        .unwrap()
        .join("_build/watch-tree-flat")
        .to_str()
        .unwrap()
        .to_owned();
    files.put(&path, 7);
    let (_session, scope) = scope(&files, &path);
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(60));
    controller.handle().set_tree_scope(scope);
    controller.set_visible(files.visible(PaneId::Left, 4, &path));
    assert_eq!(tree_changed(&mut events).await.navigation, NavigationId(4));
    stop(&controller, &mut tasks).await;
}

type BatchCallback = Arc<dyn Fn(u32) + Send + Sync>;
#[derive(Default)]
struct Batches {
    callbacks: Mutex<Vec<(Vec<PathBuf>, BatchCallback)>>,
    ready: Notify,
}
struct BatchGuard;
impl WatchSubscription for BatchGuard {}
impl WatchService for Batches {
    fn watch_directory(
        &self,
        _: &Path,
        _: ChangeCallback,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        panic!("tree scope must use one batch rather than individual subscriptions")
    }
    fn watch_directories(
        &self,
        paths: &[PathBuf],
        changed: BatchCallback,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        self.callbacks
            .lock()
            .unwrap()
            .push((paths.to_vec(), changed));
        self.ready.notify_one();
        Ok(Box::new(BatchGuard))
    }
}
impl Batches {
    async fn registered(&self) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if !self.callbacks.lock().unwrap().is_empty() {
                    break;
                }
                self.ready.notified().await;
            }
        })
        .await
        .unwrap();
    }
    fn fire(&self) {
        (self.callbacks.lock().unwrap()[0].1)(1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn callback_between_admission_and_commit_survives_ack_and_unchanged_commits() {
    let files = Arc::new(Files::default());
    let path = std::env::current_dir()
        .unwrap()
        .join("_build/watch-tree-callback")
        .to_str()
        .unwrap()
        .to_owned();
    files.put(&path, 7);
    let (_session, scope) = scope(&files, &path);
    let native = Arc::new(Batches::default());
    let (sender, mut events) = mpsc::channel(8);
    let mut tasks = JoinSet::new();
    let controller = Controller::with_timing(
        Retirement::testing(),
        TaskLifetime::testing((files.clone(), native.clone())),
        files,
        Some(native.clone()),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
        timing(Duration::from_secs(3)),
    );
    let handle = controller.handle();
    handle.set_tree_scope(scope.clone());
    native.registered().await;
    let stamp = tree_changed(&mut events).await;
    let ticket = handle.begin_refresh(stamp, TreeRequestId(1)).unwrap();
    native.fire();
    handle.finish_refresh(ticket, true, stamp.revision);
    for _ in 0..10 {
        handle.set_tree_scope(scope.clone());
    }
    assert_eq!(tree_changed(&mut events).await.session, stamp.session);
    assert_eq!(
        native.callbacks.lock().unwrap().len(),
        1,
        "unchanged commits preserve the native batch"
    );
    handle.clear_scope(PaneId::Left, NavigationId(5));
    native.fire();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err()
    );
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_scope_preserves_commit_identity_without_acknowledging_ticket() {
    let files = Arc::new(Files::default());
    let path = std::env::current_dir()
        .unwrap()
        .join("_build/watch-tree-failure")
        .to_str()
        .unwrap()
        .to_owned();
    files.put(&path, 7);
    let (_session, scope) = scope(&files, &path);
    let (controller, mut tasks, mut events) = controller(files, None, Duration::from_millis(60));
    let handle = controller.handle();
    handle.set_tree_scope(scope.clone());
    let stamp = tree_changed(&mut events).await;
    let ticket = handle.begin_refresh(stamp, TreeRequestId(1)).unwrap();
    handle.finish_refresh(ticket, true, stamp.revision);
    let error = midnight_core::TreeDiagnostic::new("scope budget exhausted");
    assert!(handle.scope_failed(stamp, Default::default(), error.clone()));
    assert!(!handle.scope_failed(stamp, Default::default(), error));
    assert_eq!(tree_changed(&mut events).await.session, stamp.session);
    controller.set_visible(VisibleDirectory {
        pane: stamp.pane,
        navigation: stamp.navigation,
        path: path.into(),
        entries: Arc::default(),
    });
    assert_eq!(tree_changed(&mut events).await.session, stamp.session);
    stop(&controller, &mut tasks).await;
}

fn expanded_scope(files: &Arc<Files>, count: usize, suffix: &str) -> (Session, tree::TreeScope) {
    let path = std::env::current_dir().unwrap().join("_build").join(suffix);
    let entries = (0..count)
        .map(|index| {
            Entry::new(
                EntryId(index as u64 + 1),
                path.join(format!("folder-{index:02}")),
                EntryKind::Directory,
            )
        })
        .collect::<Vec<_>>();
    {
        let mut directories = files.directories.lock().unwrap();
        for entry in &entries {
            directories.insert(entry.path.clone(), vec![]);
        }
        directories.insert(path.clone(), entries);
    }
    let (mut session, _) = scope(files, path.to_str().unwrap());
    for index in 0..count {
        let view = session.projection().clone();
        assert!(session.commit(&view));
        let row = view
            .rows
            .iter()
            .find(|row| row.entry.display_name == format!("folder-{index:02}"))
            .unwrap()
            .id;
        session
            .branch(
                &view,
                TreeRequestId(index as u64 + 1),
                row,
                midnight_core::TreeBranchAction::Expand,
            )
            .unwrap();
        let job = session.take_work().unwrap().unwrap();
        session.apply(job.run(), &Cancellation::default()).unwrap();
    }
    let view = session.projection().clone();
    assert!(session.commit(&view));
    let scope = session.watch_scope().unwrap().unwrap();
    (session, scope)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stable_slots_cap_one_batch_and_poll_all_remaining() {
    let files = Arc::new(Files::default());
    let (_session, scope) = expanded_scope(&files, 40, "watch-tree-many");
    let uncovered = scope.targets[40].path.clone();
    let before = files.probes.load(Ordering::SeqCst);
    let native = Arc::new(Batches::default());
    let (sender, mut events) = mpsc::channel(16);
    let mut tasks = JoinSet::new();
    let controller = Controller::with_timing(
        Retirement::testing(),
        TaskLifetime::testing((files.clone(), native.clone())),
        files.clone(),
        Some(native.clone()),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
        timing(Duration::from_millis(100)),
    );
    let handle = controller.handle();
    handle.set_tree_scope(scope.clone());
    native.registered().await;
    files.wait_probes(before + 41).await;
    let stamp = tree_changed(&mut events).await;
    let ticket = handle.begin_refresh(stamp, TreeRequestId(41)).unwrap();
    handle.finish_refresh(ticket, true, stamp.revision);
    handle.set_tree_scope(scope);
    {
        let batches = native.callbacks.lock().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].0.len(), 32);
        assert!(!batches[0].0.contains(&uncovered));
    }
    files.put(uncovered.to_str().unwrap(), 9);
    assert_eq!(tree_changed(&mut events).await.session, stamp.session);
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn folder_only_accepted_branch_with_files_is_quiet_after_success() {
    let files = Arc::new(Files::default());
    let (_session, scope) = expanded_scope(&files, 1, "watch-tree-folders");
    let branch = scope.targets[1].path.clone();
    files.put(branch.to_str().unwrap(), 12);
    let before = files.probes.load(Ordering::SeqCst);
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(60));
    let handle = controller.handle();
    handle.set_tree_scope(scope.clone());
    files.wait_probes(before + 2).await;
    let stamp = tree_changed(&mut events).await;
    let ticket = handle.begin_refresh(stamp, TreeRequestId(2)).unwrap();
    handle.finish_refresh(ticket, true, stamp.revision);
    handle.set_tree_scope(scope);
    while events.try_recv().is_ok() {}
    assert!(
        tokio::time::timeout(Duration::from_millis(240), events.recv())
            .await
            .is_err()
    );
    files.put(branch.to_str().unwrap(), 13);
    assert_eq!(tree_changed(&mut events).await.session, stamp.session);
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_probe_is_logically_cancelled_but_keeps_provider_scope_until_return() {
    let path = std::env::current_dir()
        .unwrap()
        .join("_build/watch-tree-blocked");
    let files = Arc::new(Files {
        blocked_path: Some(path.clone()),
        ignore_cancel: true,
        ..Default::default()
    });
    files.put(path.to_str().unwrap(), 7);
    let (session, scope) = scope(&files, path.to_str().unwrap());
    let stamp = scope.stamp;
    let retained = Arc::downgrade(&scope.targets[0]);
    drop(session);
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(50));
    let handle = controller.handle();
    handle.set_tree_scope(scope);
    tokio::time::timeout(Duration::from_secs(3), files.blocked.notified())
        .await
        .unwrap();
    handle.clear_scope(PaneId::Left, NavigationId(5));
    assert!(
        handle.begin_refresh(stamp, TreeRequestId(1)).is_none(),
        "logical invalidation must not wait for provider return"
    );
    assert!(
        retained.upgrade().is_some(),
        "blocked probe must retain provider ownership"
    );
    assert!(events.try_recv().is_err());
    files.release.store(true, Ordering::SeqCst);
    files.wait_probes(1).await;
    stop(&controller, &mut tasks).await;
    assert!(retained.upgrade().is_none());
    assert!(events.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_sweep_and_ignored_hint_retry_at_poll_cadence() {
    let files = Arc::new(Files::default());
    let path = std::env::current_dir()
        .unwrap()
        .join("_build/watch-tree-retry")
        .to_str()
        .unwrap()
        .to_owned();
    files.put(&path, 7);
    let (_session, scope) = scope(&files, &path);
    let (controller, mut tasks, mut events) = controller(files, None, Duration::from_millis(120));
    let handle = controller.handle();
    handle.set_tree_scope(scope);
    let stamp = tree_changed(&mut events).await;
    let ticket = handle.begin_refresh(stamp, TreeRequestId(1)).unwrap();
    handle.finish_refresh(ticket, false, stamp.revision);
    assert!(
        tokio::time::timeout(Duration::from_millis(60), events.recv())
            .await
            .is_err()
    );
    assert_eq!(tree_changed(&mut events).await.session, stamp.session);
    assert!(
        tokio::time::timeout(Duration::from_millis(60), events.recv())
            .await
            .is_err()
    );
    assert_eq!(tree_changed(&mut events).await.session, stamp.session);
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fatal_session_retirement_suppresses_callbacks_and_delayed_flat_ack() {
    let files = Arc::new(Files::default());
    let path = std::env::current_dir()
        .unwrap()
        .join("_build/watch-tree-fatal")
        .to_str()
        .unwrap()
        .to_owned();
    files.put(&path, 7);
    let (session, scope) = scope(&files, &path);
    let retained = Arc::downgrade(&scope.targets[0]);
    let native = Arc::new(Batches::default());
    let (sender, mut events) = mpsc::channel(8);
    let mut tasks = JoinSet::new();
    let controller = Controller::with_timing(
        Retirement::testing(),
        TaskLifetime::testing((files.clone(), native.clone())),
        files.clone(),
        Some(native.clone()),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
        timing(Duration::from_millis(60)),
    );
    let handle = controller.handle();
    handle.set_tree_scope(scope);
    native.registered().await;
    let queued_hint = tree_changed(&mut events).await;
    handle.retire_session(queued_hint);
    drop(session);
    native.fire();
    controller.set_visible(files.visible(PaneId::Left, 4, &path));
    assert!(
        handle
            .begin_refresh(queued_hint, TreeRequestId(1))
            .is_none()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(180), events.recv())
            .await
            .is_err()
    );
    stop(&controller, &mut tasks).await;
    assert!(retained.upgrade().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_probe_retries_after_a_successful_listing_ack_without_losing_history() {
    let files = Arc::new(Files::default());
    let path = std::env::current_dir()
        .unwrap()
        .join("_build/watch-tree-probe-failure")
        .to_str()
        .unwrap()
        .to_owned();
    files.put(&path, 7);
    let (_session, scope) = scope(&files, &path);
    files.metadata_failed.store(true, Ordering::SeqCst);
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(80));
    let handle = controller.handle();
    handle.set_tree_scope(scope.clone());
    let stamp = tree_changed(&mut events).await;
    let ticket = handle.begin_refresh(stamp, TreeRequestId(1)).unwrap();
    handle.finish_refresh(ticket, true, stamp.revision);
    handle.set_tree_scope(scope.clone());
    assert_eq!(tree_changed(&mut events).await.session, stamp.session);
    files.metadata_failed.store(false, Ordering::SeqCst);
    let probes = files.probes.load(Ordering::SeqCst);
    files.wait_probes(probes + 1).await;
    // Drain the recovery interval before acknowledging the now-readable listing.
    while events.try_recv().is_ok() {}
    // read_directory notifies before the watch worker records its observation;
    // wait for the resulting hint so this test cannot ACK ahead of recovery.
    assert_eq!(tree_changed(&mut events).await.session, stamp.session);
    let ticket = handle.begin_refresh(stamp, TreeRequestId(2)).unwrap();
    handle.finish_refresh(ticket, true, stamp.revision);
    handle.set_tree_scope(scope);
    // A slow test/event consumer may already have multiple paced hints queued.
    // Observe only future output when asserting that accepted state stays quiet.
    while events.try_recv().is_ok() {}
    assert!(
        tokio::time::timeout(Duration::from_millis(240), events.recv())
            .await
            .is_err()
    );
    stop(&controller, &mut tasks).await;
}

// Tree-only identity observations. Existing flat-watch Files/signature behavior stays unchanged.
type WatchDirectoryIdentity = midnight_core::SizeDirectoryIdentity;

#[derive(Default)]
struct LinkWatchFiles {
    files: Files,
    identities: Mutex<HashMap<PathBuf, WatchDirectoryIdentity>>,
    retarget_after_listing: Mutex<HashMap<PathBuf, WatchDirectoryIdentity>>,
    listings: Mutex<Vec<PathBuf>>,
    identity_reads: Mutex<Vec<PathBuf>>,
}

fn watch_identity(object: u128) -> WatchDirectoryIdentity {
    WatchDirectoryIdentity {
        device: 73,
        object: object.to_le_bytes(),
    }
}

impl DirectoryService for LinkWatchFiles {
    fn browse_directory_identity(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<WatchDirectoryIdentity>, FsError> {
        if cancel.is_cancelled() {
            return Err(FsError::from_io(
                "watch test identity",
                path,
                io::ErrorKind::Interrupted.into(),
            ));
        }
        self.identity_reads.lock().unwrap().push(path.to_owned());
        Ok(self.identities.lock().unwrap().get(path).copied())
    }

    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        let result = self.files.read_directory(path, cancel, emit);
        self.listings.lock().unwrap().push(path.to_owned());
        if let Some(identity) = self.retarget_after_listing.lock().unwrap().remove(path) {
            self.identities
                .lock()
                .unwrap()
                .insert(path.to_owned(), identity);
        }
        result
    }

    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }

    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        self.files.entry_metadata(path)
    }
}

impl VolumeService for LinkWatchFiles {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}

impl StorageService for LinkWatchFiles {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        Err(FsError::from_io(
            "watch test space",
            path,
            io::ErrorKind::Unsupported.into(),
        ))
    }
}

fn expand_link_watch_row(session: &mut Session, name: &str, request: u64, refresh: bool) {
    let view = session.projection().clone();
    assert!(session.commit(&view));
    let row = view
        .rows
        .iter()
        .find(|row| row.entry.display_name == name)
        .unwrap()
        .id;
    session
        .branch(
            &view,
            TreeRequestId(request),
            row,
            if refresh {
                midnight_core::TreeBranchAction::Refresh
            } else {
                midnight_core::TreeBranchAction::Expand
            },
        )
        .unwrap();
    let job = session.take_work().unwrap().unwrap();
    assert!(session.apply(job.run(), &Cancellation::default()).unwrap());
}

fn linked_watch_scope() -> (Arc<LinkWatchFiles>, Session, tree::TreeScope, Vec<PathBuf>) {
    let root = std::env::current_dir()
        .unwrap()
        .join("_build/watch-tree-linked-runtime");
    let files = Arc::new(LinkWatchFiles::default());
    let names = [
        ("direct", EntryKind::Directory),
        ("alias-a", EntryKind::DirectoryLink),
        ("alias-b", EntryKind::DirectoryLink),
    ];
    let paths = names
        .iter()
        .map(|(name, _)| root.join(name))
        .collect::<Vec<_>>();
    {
        let mut directories = files.files.directories.lock().unwrap();
        directories.insert(
            root.clone(),
            names
                .iter()
                .enumerate()
                .map(|(index, (name, kind))| {
                    Entry::new(EntryId(index as u64 + 1), root.join(name), *kind)
                })
                .collect(),
        );
        for path in &paths {
            directories.insert(path.clone(), Vec::new());
        }
    }
    {
        let mut identities = files.identities.lock().unwrap();
        identities.insert(root.clone(), watch_identity(1));
        for path in &paths {
            identities.insert(path.clone(), watch_identity(2));
        }
    }
    let visible = files.files.visible(PaneId::Left, 4, root.to_str().unwrap());
    let (mut session, _) = scope_from_visible(files.clone(), visible);
    for (index, (name, _)) in names.iter().enumerate() {
        expand_link_watch_row(&mut session, name, index as u64 + 1, false);
    }
    let view = session.projection().clone();
    assert!(session.commit(&view));
    let scope = session.watch_scope().unwrap().unwrap();
    assert_eq!(
        scope.targets.len(),
        4,
        "root plus all three logical occurrences"
    );
    assert_eq!(scope.eligible.len(), 4);
    for target in &scope.targets {
        let expected = files.identities.lock().unwrap()[&target.path];
        assert_eq!(
            target.directory_identity,
            Some(expected),
            "scope identity belongs to its acknowledged listing"
        );
    }
    (files, session, scope, paths)
}

fn link_watch_timing() -> Timing {
    Timing {
        quiet: Duration::ZERO,
        maximum: Duration::ZERO,
        poll: Duration::ZERO,
    }
}

// Drive the real tree Runtime synchronously: no background scheduling or sleeps
// decide which observation was made before an acknowledgement or callback.
struct LinkWatchRuntime {
    handle: Handle,
    runtime: tree::Runtime,
    native: Arc<Batches>,
    outbox: Outbox,
    events: mpsc::Receiver<Event>,
}

impl LinkWatchRuntime {
    fn new(scope: &tree::TreeScope) -> Self {
        let handle = Handle {
            panes: Retirement::testing().panes.map(|lease| {
                Arc::new(lease.own(Shared {
                    state: Mutex::new(Control {
                        visible: None,
                        revision: 0,
                        cancel: Cancellation::default(),
                        stopped: false,
                        navigation: NavigationId(0),
                        tree_navigation: None,
                        tree: None,
                    }),
                    worker: std::sync::OnceLock::new(),
                    timing: link_watch_timing(),
                }))
            }),
        };
        handle.set_tree_scope(scope.clone());
        let shared = &handle.panes[scope.stamp.pane.index()];
        let cancel = shared.state.lock().unwrap().cancel.clone();
        let native = Arc::new(Batches::default());
        let mut runtime = tree::Runtime::new(Arc::new(scope.clone()), cancel);
        runtime.update(Arc::new(scope.clone()), shared, Some(native.as_ref()));
        let (sender, events) = mpsc::channel(128);
        Self {
            handle,
            runtime,
            native,
            outbox: Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            events,
        }
    }

    fn tick(&mut self, files: &dyn midnight_core::FileSystem, count: usize) {
        for _ in 0..count {
            self.runtime.step(
                &self.handle.panes[PaneId::Left.index()],
                files,
                &self.outbox,
                link_watch_timing(),
            );
        }
    }

    fn changed(&mut self) -> Option<midnight_core::TreeViewStamp> {
        let mut stamp = None;
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Tree(event) => match *event {
                    TreeEvent::Changed { stamp: next } => stamp = Some(next),
                    other => panic!("unexpected linked watch event: {other:?}"),
                },
                other => panic!("unexpected linked watch event: {other:?}"),
            }
        }
        stamp
    }

    fn acknowledge(&mut self, scope: &tree::TreeScope, request: u64) {
        let ticket = self
            .handle
            .begin_refresh(scope.stamp, TreeRequestId(request))
            .unwrap();
        self.handle
            .finish_refresh(ticket, true, scope.stamp.revision);
        self.handle.set_tree_scope(scope.clone());
        self.runtime.update(
            Arc::new(scope.clone()),
            &self.handle.panes[scope.stamp.pane.index()],
            Some(self.native.as_ref()),
        );
    }

    fn settle(
        &mut self,
        files: &dyn midnight_core::FileSystem,
        scope: &tree::TreeScope,
        request: u64,
    ) {
        self.tick(files, scope.targets.len() + 1);
        self.changed();
        self.acknowledge(scope, request);
        self.tick(files, scope.targets.len() + 1);
        assert!(
            self.changed().is_none(),
            "accepted stable scope must become quiet"
        );
    }
}

#[test]
fn tree_link_watch_physical_slots_are_deduplicated_and_all_logical_paths_are_polled() {
    let (files, _session, scope, paths) = linked_watch_scope();
    let mut runtime = LinkWatchRuntime::new(&scope);
    {
        let batches = runtime.native.callbacks.lock().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0].0.len(),
            2,
            "root plus one shared physical directory"
        );
        let identities = files.identities.lock().unwrap();
        let registered = batches[0]
            .0
            .iter()
            .map(|path| identities[path])
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(registered.len(), 2);
    }
    files.listings.lock().unwrap().clear();
    runtime.settle(files.as_ref(), &scope, 100);
    for path in &paths {
        assert!(
            files.listings.lock().unwrap().contains(path),
            "logical path not polled: {path:?}"
        );
    }
    {
        let mut directories = files.files.directories.lock().unwrap();
        for path in &paths {
            directories.insert(
                path.clone(),
                vec![Entry::new(
                    EntryId(1),
                    path.join("new-child"),
                    EntryKind::Directory,
                )],
            );
        }
    }
    runtime.tick(files.as_ref(), scope.targets.len() + 1);
    assert_eq!(
        runtime.changed(),
        Some(scope.stamp),
        "empty-to-nonempty requests maintenance"
    );
    for path in &paths {
        assert!(
            !files
                .listings
                .lock()
                .unwrap()
                .contains(&path.join("new-child")),
            "watch polling must not discover recursive targets"
        );
    }
}

#[test]
fn tree_link_watch_same_path_retarget_with_identical_contents_rebinds_after_ack() {
    let (files, mut session, scope, paths) = linked_watch_scope();
    let alias = &paths[1];
    let mut runtime = LinkWatchRuntime::new(&scope);
    runtime.settle(files.as_ref(), &scope, 100);
    let old_callback = runtime.native.callbacks.lock().unwrap()[0].1.clone();
    files
        .identities
        .lock()
        .unwrap()
        .insert(alias.clone(), watch_identity(3));
    runtime.tick(files.as_ref(), scope.targets.len() + 1);
    assert_eq!(
        runtime.changed(),
        Some(scope.stamp),
        "identity-only retarget must request maintenance despite identical empty contents"
    );
    assert_eq!(runtime.native.callbacks.lock().unwrap().len(), 1);

    expand_link_watch_row(&mut session, "alias-a", 4, true);
    let view = session.projection().clone();
    assert!(session.commit(&view));
    let accepted = session.watch_scope().unwrap().unwrap();
    assert_eq!(
        accepted
            .targets
            .iter()
            .find(|target| target.path == *alias)
            .unwrap()
            .directory_identity,
        Some(watch_identity(3))
    );
    runtime.acknowledge(&accepted, 101);
    assert_eq!(runtime.native.callbacks.lock().unwrap().len(), 2);
    assert_eq!(runtime.native.callbacks.lock().unwrap()[1].0.len(), 3);
    runtime.settle(files.as_ref(), &accepted, 102);
    assert_eq!(runtime.native.callbacks.lock().unwrap().len(), 2);

    old_callback(u32::MAX);
    runtime.tick(files.as_ref(), accepted.targets.len() + 1);
    assert!(
        runtime.changed().is_none(),
        "replaced native callbacks cannot dirty the new scope"
    );
    let new_callback = runtime.native.callbacks.lock().unwrap()[1].1.clone();
    new_callback(u32::MAX);
    runtime.tick(files.as_ref(), 1);
    assert_eq!(runtime.changed(), Some(accepted.stamp));
}

#[test]
fn tree_link_watch_identity_change_during_poll_does_not_accept_a_new_baseline() {
    let (files, _session, scope, paths) = linked_watch_scope();
    let alias = &paths[1];
    let mut runtime = LinkWatchRuntime::new(&scope);
    runtime.settle(files.as_ref(), &scope, 100);
    files.identity_reads.lock().unwrap().clear();
    files
        .retarget_after_listing
        .lock()
        .unwrap()
        .insert(alias.clone(), watch_identity(3));
    runtime.tick(files.as_ref(), scope.targets.len() + 1);
    assert!(files.retarget_after_listing.lock().unwrap().is_empty());
    assert!(
        files
            .identity_reads
            .lock()
            .unwrap()
            .iter()
            .filter(|path| *path == alias)
            .count()
            >= 2,
        "identity must be observed before and after the real listing callback"
    );
    assert_eq!(runtime.changed(), Some(scope.stamp));

    // A completed maintenance ticket carrying the old accepted identity cannot
    // turn the rejected observation into a quiet baseline for the new target.
    runtime.acknowledge(&scope, 101);
    let listings = files
        .listings
        .lock()
        .unwrap()
        .iter()
        .filter(|path| *path == alias)
        .count();
    runtime.tick(files.as_ref(), scope.targets.len() + 1);
    assert_eq!(runtime.changed(), Some(scope.stamp));
    assert_eq!(
        files
            .listings
            .lock()
            .unwrap()
            .iter()
            .filter(|path| *path == alias)
            .count(),
        listings,
        "unacknowledged retarget must fail identity validation before enumeration"
    );
}

#[test]
fn tree_link_watch_unknown_identities_keep_distinct_slots_and_ordinary_polling() {
    let files = Arc::new(Files::default());
    let (_session, scope) = expanded_scope(&files, 2, "watch-tree-unknown-identities");
    assert_eq!(scope.targets.len(), 3);
    assert!(
        scope
            .targets
            .iter()
            .all(|target| target.directory_identity.is_none()),
        "ordinary providers without identity support retain the compatibility path"
    );
    let mut runtime = LinkWatchRuntime::new(&scope);
    {
        let batches = runtime.native.callbacks.lock().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0].0.len(),
            3,
            "None is not a shared physical identity"
        );
        assert_eq!(
            batches[0]
                .0
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3,
            "root and both unrelated ordinary directories need distinct slots"
        );
    }
    let before = files.probes.load(Ordering::SeqCst);
    runtime.settle(files.as_ref(), &scope, 100);
    assert!(files.probes.load(Ordering::SeqCst) >= before + scope.targets.len());
    for (index, target) in scope.targets.iter().skip(1).enumerate() {
        // File-only changes do not alter the accepted folder-only display
        // signature, so each positive hint proves that logical branch was polled.
        files.put(target.path.to_str().unwrap(), index as u64 + 7);
        runtime.tick(files.as_ref(), scope.targets.len() + 1);
        assert_eq!(
            runtime.changed(),
            Some(scope.stamp),
            "ordinary logical branch was not polled: {:?}",
            target.path
        );
        runtime.settle(files.as_ref(), &scope, index as u64 + 101);
    }
    // Neither child probe observes this immediate-root listing metadata change.
    files
        .directories
        .lock()
        .unwrap()
        .get_mut(&scope.targets[0].path)
        .unwrap()[0]
        .metadata
        .native_attributes = Some(0x10);
    runtime.tick(files.as_ref(), scope.targets.len() + 1);
    assert_eq!(
        runtime.changed(),
        Some(scope.stamp),
        "root must remain polled too"
    );
    assert_eq!(runtime.native.callbacks.lock().unwrap().len(), 1);
}
