use super::*;

#[path = "fence_tests.rs"]
mod fence_tests;
#[path = "lifetime_tests.rs"]
mod lifetime_tests;
#[path = "timing_tests.rs"]
mod timing_tests;
#[path = "tree_tests.rs"]
mod tree_tests;

#[test]
fn polling_signature_includes_native_attributes_and_unix_permissions() {
    let entry = Entry::new(EntryId(1), "root/file".into(), EntryKind::File);
    let signature = |entry: &Entry| accepted_signature(std::slice::from_ref(entry));
    let mut changed = entry.clone();
    changed.metadata.native_attributes = Some(0x2);
    assert_ne!(signature(&entry), signature(&changed));
    changed.metadata.native_attributes = None;
    changed.metadata.unix_mode = Some(0o100640);
    assert_ne!(signature(&entry), signature(&changed));
    let previous = signature(&changed);
    changed.metadata.unix_mode = Some(0o100600);
    assert_ne!(previous, signature(&changed));
}
use midnight_core::{
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
fn debouncer_waits_for_quiet_after_the_last_event() {
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
fn debouncer_caps_continuous_events_at_the_maximum_delay() {
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
fn debouncer_resets_after_a_delivered_burst() {
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

/// One step of a `Native` fake's subscription history, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Watched {
    Registered(PathBuf),
    Dropped(PathBuf),
    /// A test step, such as the UI's acceptance of a listing.
    Mark(&'static str),
}

#[derive(Default)]
struct Native {
    callbacks: Mutex<Vec<(PathBuf, ChangeCallback)>>,
    registered: Notify,
    drops: Arc<AtomicUsize>,
    dropped: Arc<Notify>,
    drop_threads: Arc<Mutex<Vec<(ThreadId, ThreadId)>>>,
    history: Arc<Mutex<Vec<Watched>>>,
    fail: bool,
    /// Fires the newest subscription when the next one is registered: a
    /// notification that arrives while the worker subscribes again.
    fire_on_watch: AtomicBool,
    /// So many of the next registrations panic after recording their callback.
    panics: AtomicUsize,
}

struct Guard {
    path: PathBuf,
    drops: Arc<AtomicUsize>,
    dropped: Arc<Notify>,
    drop_threads: Arc<Mutex<Vec<(ThreadId, ThreadId)>>>,
    history: Arc<Mutex<Vec<Watched>>>,
    registered_on: ThreadId,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.history
            .lock()
            .unwrap()
            .push(Watched::Dropped(std::mem::take(&mut self.path)));
        self.drops.fetch_add(1, Ordering::SeqCst);
        self.drop_threads
            .lock()
            .unwrap()
            .push((self.registered_on, std::thread::current().id()));
        self.dropped.notify_one();
    }
}
impl WatchSubscription for Guard {}

impl WatchService for Native {
    fn watch_directory(
        &self,
        path: &Path,
        changed: ChangeCallback,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        if self.fire_on_watch.swap(false, Ordering::SeqCst) {
            let newest = self
                .callbacks
                .lock()
                .unwrap()
                .last()
                .map(|(_, callback)| callback.clone());
            if let Some(callback) = newest {
                callback();
            }
        }
        let panics = self
            .panics
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok();
        // Recorded before `registrations` can observe this registration.
        if !panics && !self.fail {
            self.history
                .lock()
                .unwrap()
                .push(Watched::Registered(path.into()));
        }
        self.callbacks.lock().unwrap().push((path.into(), changed));
        self.registered.notify_one();
        if panics {
            panic!("watch test registration panic");
        }
        if self.fail {
            return Err(FsError::from_io(
                "test watch",
                path,
                io::ErrorKind::Unsupported.into(),
            ));
        }
        Ok(Box::new(Guard {
            path: path.into(),
            drops: self.drops.clone(),
            dropped: self.dropped.clone(),
            drop_threads: self.drop_threads.clone(),
            history: self.history.clone(),
            registered_on: std::thread::current().id(),
        }))
    }
}

impl Native {
    fn mark(&self, step: &'static str) {
        self.history.lock().unwrap().push(Watched::Mark(step));
    }

    fn history(&self) -> Vec<Watched> {
        self.history.lock().unwrap().clone()
    }

    async fn registrations(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
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

    async fn released(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.drops.load(Ordering::SeqCst) < count {
                self.dropped.notified().await;
            }
        })
        .await
        .expect("Watch subscription must be released on its worker");
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
    ignore_cancel: bool,
    metadata_failed: AtomicBool,
    /// The watched directory itself cannot be listed.
    listing_failed: AtomicBool,
    /// Enumeration reports complete listing metadata, as Windows and the plugin do.
    complete: bool,
    metadata_reads: AtomicUsize,
    /// So many of the next directory reads panic.
    panic_probes: AtomicUsize,
}

/// Lets the fake's blocked probe return when dropped. Hold one for as long as a test
/// keeps a probe blocked: a failed assertion then still releases the spinning blocking
/// thread instead of hanging the runtime's shutdown forever.
struct ReleaseOnDrop(Arc<Files>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release.store(true, Ordering::SeqCst);
    }
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
        tokio::time::timeout(Duration::from_secs(10), async {
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
        if self
            .panic_probes
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            panic!("watch test probe panic");
        }
        if self.blocked_path.as_deref() == Some(path) {
            self.blocked.notify_one();
            while (self.ignore_cancel || !cancel.is_cancelled())
                && !self.release.load(Ordering::SeqCst)
            {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        if self.listing_failed.load(Ordering::SeqCst) {
            self.probes.fetch_add(1, Ordering::SeqCst);
            self.probed.notify_one();
            return Err(FsError::from_io(
                "test listing",
                path,
                io::ErrorKind::PermissionDenied.into(),
            ));
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

    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        self.complete
    }

    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        self.metadata_reads.fetch_add(1, Ordering::SeqCst);
        if self.metadata_failed.load(Ordering::SeqCst) {
            return Err(FsError::from_io(
                "test metadata",
                path,
                io::ErrorKind::PermissionDenied.into(),
            ));
        }
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
        restart: Duration::from_secs(60),
    }
}

fn controller(
    files: Arc<Files>,
    native: Option<Arc<Native>>,
    poll: Duration,
) -> (Controller, JoinSet<()>, mpsc::Receiver<Event>) {
    controller_with(files, native, timing(poll), 8)
}

fn controller_with(
    files: Arc<Files>,
    native: Option<Arc<Native>>,
    timing: Timing,
    capacity: usize,
) -> (Controller, JoinSet<()>, mpsc::Receiver<Event>) {
    let (sender, events) = mpsc::channel(capacity);
    let mut tasks = JoinSet::new();
    let controller = Controller::with_timing(
        Retirement::testing(),
        TaskLifetime::testing((files.clone(), native.clone())),
        files,
        native.map(|native| native as Arc<dyn WatchService>),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
        timing,
    );
    (controller, tasks, events)
}

async fn next_event(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("Watch must emit an event")
        .expect("Watch output closed")
}

async fn changed(events: &mut mpsc::Receiver<Event>) -> (PaneId, NavigationId) {
    match next_event(events).await {
        Event::DirectoryChanged { pane, navigation } => (pane, navigation),
        other => panic!("Unexpected watch event: {other:?}"),
    }
}

/// The kind of a reported status, for comparisons.
fn kind(status: &WatchStatus) -> &'static str {
    match status {
        WatchStatus::Native => "Native",
        WatchStatus::PollingOnly(_) => "PollingOnly",
        WatchStatus::Restarting => "Restarting",
    }
}

/// The next event, which must be a watch status.
async fn status(events: &mut mpsc::Receiver<Event>) -> (WatchedView, WatchStatus) {
    match next_event(events).await {
        Event::WatchStatus { view, status } => (view, status),
        other => panic!("Expected a watch status: {other:?}"),
    }
}

fn flat(pane: PaneId, navigation: u64) -> WatchedView {
    WatchedView::Flat {
        pane,
        navigation: NavigationId(navigation),
    }
}

async fn stop(controller: &Controller, tasks: &mut JoinSet<()>) {
    controller.shutdown();
    tokio::time::timeout(Duration::from_secs(10), async {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistently_unreadable_entry_metadata_stays_quiet_after_acceptance() {
    // Names are enumerable but every stat fails (an `r--` Linux directory, a
    // failing FUSE entry or a Windows trailing-dot name). The listing keeps the
    // row with unknown metadata, so the polling probe must do the same.
    let files = Arc::new(Files::default());
    files.put("left", 7);
    files
        .directories
        .lock()
        .unwrap()
        .get_mut(Path::new("left"))
        .unwrap()[0]
        .metadata = EntryMetadata::default();
    files.metadata_failed.store(true, Ordering::SeqCst);
    let visible = files.visible(PaneId::Left, 1, "left");
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(30));
    controller.set_visible(visible);
    files.wait_probes(4).await;
    assert!(
        events.try_recv().is_err(),
        "an unreadable but unchanged entry must not refresh every poll"
    );
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn complete_enumeration_metadata_is_compared_without_per_entry_stats() {
    let files = Arc::new(Files {
        complete: true,
        ..Files::default()
    });
    files.put("left", 7);
    let (controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(30));
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    files.wait_probes(2).await;
    assert!(events.try_recv().is_err());
    assert_eq!(files.metadata_reads.load(Ordering::SeqCst), 0);
    files.put("left", 8);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    assert_eq!(files.metadata_reads.load(Ordering::SeqCst), 0);
    stop(&controller, &mut tasks).await;
}

async fn assert_polling_observes_compression_only_change(
    initial: midnight_core::CompressionStatus,
) {
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
        entry.metadata.compression = midnight_core::CompressionStatus::Compressed;
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
    assert_polling_observes_compression_only_change(midnight_core::CompressionStatus::Uncompressed)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn polling_refreshes_recovered_compression_without_length_or_mtime_changes() {
    assert_polling_observes_compression_only_change(midnight_core::CompressionStatus::Unknown)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_native_registration_reports_polling_only_once_and_keeps_polling() {
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
    let (view, reported) = status(&mut events).await;
    assert_eq!(view, flat(PaneId::Left, 1));
    let WatchStatus::PollingOnly(error) = reported else {
        panic!("expected polling only: {reported:?}");
    };
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    // Only the periodic check finds the change; the status is not repeated.
    files.put("left", 8);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    // A refresh of the same folder is a new view: its failure is reported once.
    controller.listing_started(PaneId::Left, NavigationId(2), Path::new("left"));
    controller
        .handle()
        .clear_scope(PaneId::Left, NavigationId(2));
    controller.set_visible(files.visible(PaneId::Left, 2, "left"));
    let reported = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match next_event(&mut events).await {
                Event::WatchStatus { view, status } => break (view, kind(&status)),
                // A hint for the replaced rows may still be queued.
                Event::DirectoryChanged {
                    navigation: NavigationId(1),
                    ..
                } => {}
                other => panic!("Unexpected watch event: {other:?}"),
            }
        }
    })
    .await
    .expect("the refreshed view's status must be reported");
    assert_eq!(reported, (flat(PaneId::Left, 2), "PollingOnly"));
    stop(&controller, &mut tasks).await;
    assert!(
        events.try_recv().is_err(),
        "each view's status is sent once"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_status_waiting_for_capacity_is_sent_while_a_failed_navigation_shows_its_rows() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let native = Arc::new(Native {
        fail: true,
        ..Native::default()
    });
    // A full queue keeps the status of navigation 1 unsent.
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
        Retirement::testing(),
        TaskLifetime::testing((files.clone(), native.clone())),
        files.clone(),
        Some(native.clone()),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
        timing(Duration::from_secs(30)),
    );
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    // The pane lists another folder, which fails: it still shows navigation 1.
    controller.listing_started(PaneId::Left, NavigationId(2), Path::new("right"));
    controller
        .handle()
        .clear_scope(PaneId::Left, NavigationId(2));
    controller
        .reporter()
        .listing_failed(PaneId::Left, NavigationId(2));
    assert!(matches!(
        next_event(&mut events).await,
        Event::DirectoryChanged {
            navigation: NavigationId(999),
            ..
        }
    ));
    let (view, reported) = status(&mut events).await;
    assert_eq!(
        (view, kind(&reported)),
        (flat(PaneId::Left, 1), "PollingOnly")
    );
    stop(&controller, &mut tasks).await;
}

fn restarts(controller: &Controller, pane: PaneId) -> Vec<Instant> {
    controller.panes[pane.index()]
        .restarts
        .lock()
        .unwrap()
        .clone()
}

/// Statuses of `view` until (and including) one of `last` kind.
async fn statuses_until(
    events: &mut mpsc::Receiver<Event>,
    view: WatchedView,
    last: &str,
) -> Vec<&'static str> {
    let mut reported = Vec::new();
    while reported.last().is_none_or(|kind| *kind != last) {
        let (reported_view, status) = status(events).await;
        assert_eq!(reported_view, view);
        reported.push(kind(&status));
        assert!(reported.len() <= 8, "statuses flip: {reported:?}");
    }
    reported
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panicking_probe_restarts_the_worker_at_once_then_at_most_once_per_interval() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    files.panic_probes.store(2, Ordering::SeqCst);
    let native = Arc::new(Native::default());
    let restart = Duration::from_millis(300);
    let (controller, mut tasks, mut events) = controller_with(
        files.clone(),
        Some(native.clone()),
        Timing {
            restart,
            ..timing(Duration::from_millis(30))
        },
        8,
    );
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    // The first restart is immediate and reports nothing. The second waits for
    // the interval, and the outcome of its registration is reported once its
    // first probe completed.
    assert_eq!(
        statuses_until(&mut events, flat(PaneId::Left, 1), "Native").await,
        ["Restarting", "Native"]
    );
    let restarted = restarts(&controller, PaneId::Left);
    assert_eq!(restarted.len(), 2);
    assert!(restarted[1] - restarted[0] >= restart, "{restarted:?}");
    assert_eq!(native.callbacks.lock().unwrap().len(), 3);
    assert_eq!(
        native.drops.load(Ordering::SeqCst),
        2,
        "unwound subscriptions"
    );
    // Polling continues after the restarts.
    files.put("left", 8);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    stop(&controller, &mut tasks).await;
    assert_eq!(native.drops.load(Ordering::SeqCst), 3);
    assert!(
        native
            .drop_threads
            .lock()
            .unwrap()
            .iter()
            .all(|(start, end)| start == end),
        "every subscription is released on the worker that registered it"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panicking_registration_restarts_at_once_without_a_status() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let native = Arc::new(Native {
        panics: AtomicUsize::new(1),
        ..Native::default()
    });
    let (controller, mut tasks, mut events) =
        controller(files.clone(), Some(native.clone()), Duration::from_secs(30));
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(2).await;
    native.fire(1);
    // `changed` fails on any status.
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    assert_eq!(restarts(&controller, PaneId::Left).len(), 1);
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_registration_that_keeps_panicking_is_only_polled() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let native = Arc::new(Native {
        panics: AtomicUsize::new(usize::MAX),
        ..Native::default()
    });
    // The second panic must come before the second restart is due, or that
    // restart is immediate and reports nothing; it follows the first restart
    // at once, so only an implausibly slow worker could miss 300 ms.
    let (controller, mut tasks, mut events) = controller_with(
        files.clone(),
        Some(native.clone()),
        Timing {
            restart: Duration::from_millis(300),
            ..timing(Duration::from_millis(40))
        },
        8,
    );
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    assert_eq!(
        statuses_until(&mut events, flat(PaneId::Left, 1), "PollingOnly").await,
        ["Restarting", "PollingOnly"]
    );
    assert_eq!(
        native.callbacks.lock().unwrap().len(),
        2,
        "the registration panicked twice and was then skipped"
    );
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
    let _release = ReleaseOnDrop(files.clone());
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
        Retirement::testing(),
        TaskLifetime::testing((files.clone(), native.clone())),
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
