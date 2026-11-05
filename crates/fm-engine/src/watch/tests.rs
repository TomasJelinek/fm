use super::*;
use fm_core::{
    Cancellation, DirectoryService, Entry, EntryId, EntryKind, EntryMetadata, Event, FsError,
    NavigationId, PaneId, SpaceInfo, StorageService, Volume, VolumeService, WatchSubscription,
};
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::ThreadId,
    time::SystemTime,
};
use tokio::sync::{Notify, mpsc};

#[test]
fn native_burst_waits_for_one_second_of_quiet() {
    let start = Instant::now();
    let quiet = Duration::from_secs(1);
    let maximum = Duration::from_secs(3);
    let mut debounce = Debouncer::default();
    debounce.record(start);
    debounce.record(start + Duration::from_millis(400));
    assert_eq!(
        debounce.deadline(quiet, maximum),
        Some(start + Duration::from_millis(1400))
    );
    assert!(!debounce.take_due(start + Duration::from_millis(1399), quiet, maximum));
    assert!(debounce.take_due(start + Duration::from_millis(1400), quiet, maximum));
    assert!(debounce.deadline(quiet, maximum).is_none());
}

#[test]
fn continuous_native_events_cannot_defer_refresh_beyond_three_seconds() {
    let start = Instant::now();
    let quiet = Duration::from_secs(1);
    let maximum = Duration::from_secs(3);
    let mut debounce = Debouncer::default();
    for offset in [0, 800, 1600, 2400, 2900] {
        debounce.record(start + Duration::from_millis(offset));
    }
    assert_eq!(debounce.deadline(quiet, maximum), Some(start + maximum));
    assert!(!debounce.take_due(start + Duration::from_millis(2999), quiet, maximum));
    assert!(debounce.take_due(start + maximum, quiet, maximum));
}

#[test]
fn a_delivered_burst_does_not_leave_a_permanent_dirty_latch() {
    let start = Instant::now();
    let quiet = Duration::from_secs(1);
    let maximum = Duration::from_secs(3);
    let mut debounce = Debouncer::default();
    debounce.record(start);
    assert!(debounce.take_due(start + quiet, quiet, maximum));
    assert!(!debounce.take_due(start + maximum, quiet, maximum));
    debounce.record(start + Duration::from_secs(5));
    assert!(debounce.take_due(start + Duration::from_secs(6), quiet, maximum));
}

type ChangeCallback = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Native {
    callbacks: Mutex<Vec<(PathBuf, ChangeCallback)>>,
    registered: Notify,
    drops: Arc<AtomicUsize>,
    drop_threads: Arc<Mutex<Vec<(ThreadId, ThreadId)>>>,
    fail: bool,
}

struct Guard {
    drops: Arc<AtomicUsize>,
    drop_threads: Arc<Mutex<Vec<(ThreadId, ThreadId)>>>,
    registered_on: ThreadId,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        self.drop_threads
            .lock()
            .unwrap()
            .push((self.registered_on, std::thread::current().id()));
    }
}
impl WatchSubscription for Guard {}

impl WatchService for Native {
    fn watch_directory(
        &self,
        path: &Path,
        changed: ChangeCallback,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        self.callbacks.lock().unwrap().push((path.into(), changed));
        self.registered.notify_one();
        if self.fail {
            return Err(FsError::from_io(
                "test watch",
                path,
                io::ErrorKind::Unsupported.into(),
            ));
        }
        Ok(Box::new(Guard {
            drops: self.drops.clone(),
            drop_threads: self.drop_threads.clone(),
            registered_on: std::thread::current().id(),
        }))
    }
}

impl Native {
    async fn registrations(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if self.callbacks.lock().unwrap().len() >= count {
                    return;
                }
                self.registered.notified().await;
            }
        })
        .await
        .expect("Watch registration must run on a background worker");
    }

    fn fire(&self, index: usize) {
        let callback = self.callbacks.lock().unwrap()[index].1.clone();
        callback();
    }
}

#[derive(Default)]
struct Files {
    directories: Mutex<HashMap<PathBuf, Vec<Entry>>>,
    probes: AtomicUsize,
    probed: Notify,
    blocked_path: Option<PathBuf>,
    blocked: Notify,
    release: std::sync::atomic::AtomicBool,
}

impl Files {
    fn put(&self, path: &str, length: u64) {
        let mut entry = Entry::new(
            EntryId(1),
            Path::new(path).join("文档.txt"),
            EntryKind::File,
        );
        entry.metadata = EntryMetadata {
            size: Some(length),
            modified: Some(SystemTime::UNIX_EPOCH),
            ..EntryMetadata::default()
        };
        self.directories
            .lock()
            .unwrap()
            .insert(path.into(), vec![entry]);
    }

    fn visible(&self, pane: PaneId, navigation: u64, path: &str) -> VisibleDirectory {
        VisibleDirectory {
            pane,
            navigation: NavigationId(navigation),
            path: path.into(),
            entries: self
                .directories
                .lock()
                .unwrap()
                .get(Path::new(path))
                .cloned()
                .unwrap_or_default()
                .into(),
        }
    }

    async fn wait_probes(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if self.probes.load(Ordering::SeqCst) >= count {
                    return;
                }
                self.probed.notified().await;
            }
        })
        .await
        .expect("Polling fallback must inspect visible directory");
    }
}

impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if self.blocked_path.as_deref() == Some(path) {
            self.blocked.notify_one();
            while !cancel.is_cancelled() && !self.release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let entries = self
            .directories
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .unwrap_or_default();
        for entry in entries {
            if cancel.is_cancelled() || !emit(entry) {
                break;
            }
        }
        self.probes.fetch_add(1, Ordering::SeqCst);
        self.probed.notify_one();
        Ok(())
    }

    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        self.directories
            .lock()
            .unwrap()
            .values()
            .flatten()
            .find(|entry| entry.path == path)
            .map(|entry| entry.metadata.clone())
            .ok_or_else(|| FsError::from_io("test metadata", path, io::ErrorKind::NotFound.into()))
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        unreachable!()
    }
}
impl StorageService for Files {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        unreachable!()
    }
}

fn timing(poll: Duration) -> Timing {
    Timing {
        quiet: Duration::from_millis(20),
        maximum: Duration::from_millis(70),
        poll,
    }
}

fn controller(
    files: Arc<Files>,
    native: Option<Arc<Native>>,
    poll: Duration,
) -> (Controller, JoinSet<()>, mpsc::Receiver<Event>) {
    let (sender, events) = mpsc::channel(8);
    let mut tasks = JoinSet::new();
    let controller = Controller::with_timing(
        files,
        native.map(|native| native as Arc<dyn WatchService>),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
        timing(poll),
    );
    (controller, tasks, events)
}

async fn changed(events: &mut mpsc::Receiver<Event>) -> (PaneId, NavigationId) {
    match tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .expect("Watch must emit a refresh hint")
        .expect("Watch output closed")
    {
        Event::DirectoryChanged { pane, navigation } => (pane, navigation),
        other => panic!("Unexpected watch event: {other:?}"),
    }
}

async fn stop(controller: &Controller, tasks: &mut JoinSet<()>) {
    controller.shutdown();
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .expect("Watch workers must release their subscriptions on shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_native_burst_is_coalesced_and_later_bursts_still_refresh() {
    let files = Arc::new(Files::default());
    let native = Arc::new(Native::default());
    let (controller, mut tasks, mut events) =
        controller(files.clone(), Some(native.clone()), Duration::from_secs(5));
    controller.set_visible(files.visible(PaneId::Left, 4, "left"));
    native.registrations(1).await;
    for _ in 0..50 {
        native.fire(0);
    }
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(4)));
    assert!(
        tokio::time::timeout(Duration::from_millis(80), events.recv())
            .await
            .is_err()
    );
    native.fire(0);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(4)));
    stop(&controller, &mut tasks).await;
    assert_eq!(native.drops.load(Ordering::SeqCst), 1);
    assert!(
        native
            .drop_threads
            .lock()
            .unwrap()
            .iter()
            .all(|(start, end)| start == end)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switching_target_ignores_old_callbacks_and_preserves_new_navigation_identity() {
    let files = Arc::new(Files::default());
    let native = Arc::new(Native::default());
    let (controller, mut tasks, mut events) =
        controller(files.clone(), Some(native.clone()), Duration::from_secs(5));
    controller.set_visible(files.visible(PaneId::Left, 1, "old"));
    native.registrations(1).await;
    controller.set_visible(files.visible(PaneId::Left, 2, "new"));
    native.registrations(2).await;
    native.fire(0);
    assert!(
        tokio::time::timeout(Duration::from_millis(60), events.recv())
            .await
            .is_err()
    );
    native.fire(1);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(2)));
    stop(&controller, &mut tasks).await;
    assert_eq!(native.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn panes_watching_the_same_path_receive_independent_generation_hints() {
    let files = Arc::new(Files::default());
    let native = Arc::new(Native::default());
    let (controller, mut tasks, mut events) =
        controller(files.clone(), Some(native.clone()), Duration::from_secs(5));
    controller.set_visible(files.visible(PaneId::Left, 3, "same"));
    controller.set_visible(files.visible(PaneId::Right, 8, "same"));
    native.registrations(2).await;
    native.fire(0);
    native.fire(1);
    let first = changed(&mut events).await;
    let second = changed(&mut events).await;
    assert!(first != second);
    assert!([first, second].contains(&(PaneId::Left, NavigationId(3))));
    assert!([first, second].contains(&(PaneId::Right, NavigationId(8))));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unchanged_poll_is_quiet_and_failed_refresh_hints_are_retried_until_acknowledged() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(40));
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    files.wait_probes(2).await;
    assert!(events.try_recv().is_err());
    files.put("left", 8);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    controller.set_visible(files.visible(PaneId::Left, 2, "left"));
    let probes = files.probes.load(Ordering::SeqCst);
    files.wait_probes(probes + 2).await;
    while let Ok(event) = events.try_recv() {
        assert!(matches!(
            event,
            Event::DirectoryChanged {
                navigation: NavigationId(1),
                ..
            }
        ));
    }
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovered_metadata_refreshes_once_and_stable_unknown_metadata_stays_quiet() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    files
        .directories
        .lock()
        .unwrap()
        .get_mut(Path::new("left"))
        .unwrap()[0]
        .metadata = EntryMetadata::default();
    let visible = files.visible(PaneId::Left, 1, "left");
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(40));
    controller.set_visible(visible);
    files.wait_probes(2).await;
    assert!(events.try_recv().is_err());
    // Recovery from a failed initial metadata lookup must reach the UI rather
    // than silently changing only the watcher's private baseline.
    files.put("left", 7);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    controller.set_visible(files.visible(PaneId::Left, 2, "left"));
    let probes = files.probes.load(Ordering::SeqCst);
    files.wait_probes(probes + 2).await;
    while let Ok(event) = events.try_recv() {
        assert!(matches!(
            event,
            Event::DirectoryChanged {
                navigation: NavigationId(1),
                ..
            }
        ));
    }
    files.put("left", 8);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(2)));
    stop(&controller, &mut tasks).await;
}

async fn assert_polling_observes_compression_only_change(initial: fm_core::CompressionStatus) {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    files
        .directories
        .lock()
        .unwrap()
        .get_mut(Path::new("left"))
        .unwrap()[0]
        .metadata
        .compression = initial;
    let original = files.visible(PaneId::Left, 1, "left");
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(30));
    controller.set_visible(original.clone());
    files.wait_probes(2).await;
    assert!(
        events.try_recv().is_err(),
        "unchanged compression must remain quiet"
    );
    {
        let mut directories = files.directories.lock().unwrap();
        let entry = &mut directories.get_mut(Path::new("left")).unwrap()[0];
        entry.metadata.compression = fm_core::CompressionStatus::Compressed;
        assert_eq!(entry.metadata.size, original.entries[0].metadata.size);
        assert_eq!(
            entry.metadata.modified,
            original.entries[0].metadata.modified
        );
    }
    // There is deliberately no native event: only the bounded polling fallback
    // can notice this compression-only transition and request a badge refresh.
    let change = tokio::time::timeout(Duration::from_secs(3), events.recv()).await;
    stop(&controller, &mut tasks).await;
    assert!(
        matches!(
            change,
            Ok(Some(Event::DirectoryChanged {
                pane: PaneId::Left,
                navigation: NavigationId(1),
            }))
        ),
        "polling missed compression-only change from {initial:?}: {change:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn polling_refreshes_a_compression_only_change_without_length_or_mtime_changes() {
    assert_polling_observes_compression_only_change(fm_core::CompressionStatus::Uncompressed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn polling_refreshes_recovered_compression_without_length_or_mtime_changes() {
    assert_polling_observes_compression_only_change(fm_core::CompressionStatus::Unknown).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_native_registration_retains_polling_fallback() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let native = Arc::new(Native {
        fail: true,
        ..Native::default()
    });
    let (controller, mut tasks, mut events) = controller(
        files.clone(),
        Some(native.clone()),
        Duration::from_millis(40),
    );
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    files.put("left", 8);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_blocked_probe_cannot_starve_the_other_pane() {
    let files = Arc::new(Files {
        blocked_path: Some("left".into()),
        ..Files::default()
    });
    files.put("right", 7);
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(30));
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    controller.set_visible(files.visible(PaneId::Right, 9, "right"));
    tokio::time::timeout(Duration::from_secs(3), files.blocked.notified())
        .await
        .unwrap();
    files.put("right", 8);
    assert_eq!(changed(&mut events).await, (PaneId::Right, NavigationId(9)));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_releases_full_output_queue_waiters_and_ignores_late_callbacks() {
    let files = Arc::new(Files::default());
    let native = Arc::new(Native::default());
    let (sender, mut events) = mpsc::channel(1);
    sender
        .send(Event::DirectoryChanged {
            pane: PaneId::Right,
            navigation: NavigationId(999),
        })
        .await
        .unwrap();
    let mut tasks = JoinSet::new();
    let controller = Controller::with_timing(
        files.clone(),
        Some(native.clone()),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
        timing(Duration::from_secs(5)),
    );
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    native.fire(0);
    tokio::time::sleep(Duration::from_millis(50)).await;
    stop(&controller, &mut tasks).await;
    native.fire(0);
    assert!(matches!(
        events.try_recv(),
        Ok(Event::DirectoryChanged {
            navigation: NavigationId(999),
            ..
        })
    ));
    assert!(events.try_recv().is_err());
    assert_eq!(native.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computed_folder_totals_do_not_look_like_filesystem_changes() {
    let files = Arc::new(Files::default());
    let mut directory = Entry::new(EntryId(1), "left/folder".into(), EntryKind::Directory);
    directory.metadata.modified = Some(SystemTime::UNIX_EPOCH);
    files
        .directories
        .lock()
        .unwrap()
        .insert("left".into(), vec![directory]);
    let mut visible = files.visible(PaneId::Left, 1, "left");
    Arc::make_mut(&mut visible.entries)[0].metadata.size = Some(8_192);
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(40));
    controller.set_visible(visible);
    files.wait_probes(2).await;
    assert!(events.try_recv().is_err());
    files
        .directories
        .lock()
        .unwrap()
        .get_mut(Path::new("left"))
        .unwrap()[0]
        .metadata
        .modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn continuous_native_notifications_eventually_emit_without_waiting_for_the_writer() {
    let files = Arc::new(Files::default());
    let native = Arc::new(Native::default());
    let (controller, mut tasks, mut events) =
        controller(files.clone(), Some(native.clone()), Duration::from_secs(5));
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    let keep_writing = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let writer_flag = keep_writing.clone();
    let writer = tokio::spawn(async move {
        while writer_flag.load(Ordering::SeqCst) {
            native.fire(0);
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    assert!(keep_writing.swap(false, Ordering::SeqCst));
    writer.await.unwrap();
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delayed_stale_visibility_ack_cannot_replace_the_current_target() {
    let files = Arc::new(Files::default());
    let native = Arc::new(Native::default());
    let (controller, mut tasks, mut events) =
        controller(files.clone(), Some(native.clone()), Duration::from_secs(5));
    controller.set_visible(files.visible(PaneId::Left, 9, "new"));
    native.registrations(1).await;
    controller.set_visible(files.visible(PaneId::Left, 8, "old"));
    native.fire(0);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(9)));
    assert_eq!(native.callbacks.lock().unwrap().len(), 1);
    stop(&controller, &mut tasks).await;
}
