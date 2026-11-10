use super::*;
use crate::{
    providers::NativeProvider,
    tree_actor::{Session, WorkerBudget},
    tree_projection::ProjectionOptions,
};
use midnight_core::{TreeEvent, TreeRequestId, TreeRootSnapshot};

fn scope(files: &Arc<Files>, path: &str) -> (Session, tree::TreeScope) {
    let visible = files.visible(PaneId::Left, 4, path);
    let root = TreeRootSnapshot {
        pane: visible.pane,
        navigation: visible.navigation,
        index: Arc::new(midnight_core::DirectoryIndex::from_entries(
            &visible.entries,
        )),
        entries: visible.entries,
    };
    let native = NativeProvider::new(files.clone());
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
