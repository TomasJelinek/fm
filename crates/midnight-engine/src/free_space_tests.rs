use super::*;
use midnight_core::{
    DirectoryService, Entry, EntryMetadata, StorageService, Volume, VolumeService,
};
use std::{
    path::Path,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
use tokio::sync::mpsc;

const TIMING: Timing = Timing {
    throttle: Duration::from_secs(1),
    local_throttle: Duration::from_millis(100),
    follow: Duration::from_millis(1_500),
    periodic: Duration::from_secs(5),
};

/// A volume whose free space a test changes, optionally holding each read until
/// the test opens the gate.
struct Disk {
    kind: StorageKind,
    available: AtomicU64,
    reads: AtomicUsize,
    active: AtomicUsize,
    maximum: AtomicUsize,
    held: Mutex<bool>,
    gate: Condvar,
}

impl Disk {
    fn new(kind: StorageKind) -> Arc<Self> {
        Arc::new(Self {
            kind,
            available: AtomicU64::new(100),
            reads: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
            held: Mutex::new(false),
            gate: Condvar::new(),
        })
    }
    fn hold(&self, held: bool) {
        *self.held.lock().unwrap() = held;
        self.gate.notify_all();
    }
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl DirectoryService for Disk {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        unreachable!()
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        unreachable!()
    }
}
impl VolumeService for Disk {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        unreachable!()
    }
}
impl StorageService for Disk {
    fn storage_kind(&self, _: &Path) -> Result<StorageKind, FsError> {
        Ok(self.kind)
    }
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        let mut held = self.held.lock().unwrap();
        while *held {
            held = self.gate.wait(held).unwrap();
        }
        drop(held);
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(SpaceInfo {
            available: self.available.load(Ordering::SeqCst),
            total: 1000,
        })
    }
}

struct Harness {
    disk: Arc<Disk>,
    controller: Controller,
    events: mpsc::Receiver<Event>,
    tasks: JoinSet<()>,
}

impl Harness {
    fn new(kind: StorageKind) -> Self {
        Self::with_timing(kind, TIMING)
    }

    fn with_timing(kind: StorageKind, timing: Timing) -> Self {
        let disk = Disk::new(kind);
        let (sender, events) = mpsc::channel(16);
        let mut tasks = JoinSet::new();
        let controller = Controller::with_timing(
            TaskLifetime::testing(disk.clone()),
            disk.clone(),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            Arc::new(Semaphore::new(2)),
            timing,
            &mut tasks,
        );
        Self {
            disk,
            controller,
            events,
            tasks,
        }
    }

    fn navigate(&self, navigation: u64) -> Cancellation {
        let cancel = Cancellation::default();
        self.controller.navigate(
            PaneId::Left,
            NavigationId(navigation),
            PathBuf::from(format!("/folder{navigation}")),
            cancel.clone(),
        );
        cancel
    }

    /// The next free space the controller reports, skipping storage kinds.
    async fn space(&mut self) -> (NavigationId, u64) {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(60), self.events.recv())
                .await
                .expect("a free-space event")
                .expect("an open event queue");
            if let Event::SpaceUpdated {
                pane,
                navigation,
                space,
            } = event
            {
                assert_eq!(pane, PaneId::Left);
                return (navigation, space.available);
            }
        }
    }

    /// Lets every due read run, `wait` of virtual time long.
    async fn idle(&self, wait: Duration) {
        tokio::time::sleep(wait).await;
    }

    fn spaces_sent(&mut self) -> Vec<(NavigationId, u64)> {
        let mut sent = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            if let Event::SpaceUpdated {
                navigation, space, ..
            } = event
            {
                sent.push((navigation, space.available));
            }
        }
        sent
    }

    /// Waits, without timers, until `reads` reads have entered the held volume. A
    /// paused clock stays put while a blocking read runs, so a timer would never fire.
    async fn entered(&self, reads: usize) {
        let started = std::time::Instant::now();
        while self.disk.reads() < reads || self.disk.active.load(Ordering::SeqCst) == 0 {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(30),
                "no read entered"
            );
            tokio::task::yield_now().await;
        }
    }

    async fn shutdown(mut self) {
        self.disk.hold(false);
        drop(self.controller);
        while self.tasks.join_next().await.is_some() {}
    }
}

#[tokio::test(start_paused = true)]
async fn a_requested_read_reports_the_changed_value_once_the_throttle_passed() {
    let mut harness = Harness::new(StorageKind::Network);
    harness.navigate(1);
    assert_eq!(harness.space().await, (NavigationId(1), 100));
    let read = Instant::now();

    // A job wrote to the volume.
    harness.disk.available.store(70, Ordering::SeqCst);
    harness.controller.refresh(PaneId::Left, NavigationId(1));
    assert_eq!(harness.space().await, (NavigationId(1), 70));
    let waited = read.elapsed();
    assert!(waited >= TIMING.throttle, "{waited:?}");
    assert!(
        waited < TIMING.throttle + Duration::from_millis(100),
        "{waited:?}"
    );
    assert_eq!(harness.disk.reads(), 2);
    harness.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn many_requests_keep_one_read_in_flight_and_owe_one_trailing_read() {
    let mut harness = Harness::new(StorageKind::Network);
    harness.navigate(1);
    assert_eq!(harness.space().await, (NavigationId(1), 100));

    harness.disk.hold(true);
    harness.controller.refresh(PaneId::Left, NavigationId(1));
    // A held read stops the paused clock from advancing by itself.
    tokio::time::advance(TIMING.throttle).await;
    harness.entered(2).await;
    // A share that answers slowly while a job keeps writing to it.
    for _ in 0..100 {
        harness.controller.refresh(PaneId::Left, NavigationId(1));
        tokio::task::yield_now().await;
    }
    harness.disk.available.store(50, Ordering::SeqCst);
    harness.disk.hold(false);
    assert_eq!(harness.space().await, (NavigationId(1), 50));
    // The last change is still read, once, a throttle after the slow read ended.
    harness.disk.available.store(40, Ordering::SeqCst);
    assert_eq!(harness.space().await, (NavigationId(1), 40));
    harness.idle(TIMING.periodic * 4).await;
    assert_eq!(harness.disk.reads(), 3);
    assert_eq!(harness.disk.maximum.load(Ordering::SeqCst), 1);
    assert!(harness.spaces_sent().is_empty());
    harness.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_superseded_navigation_drops_its_answer_and_its_requests() {
    let mut harness = Harness::new(StorageKind::Network);
    harness.disk.hold(true);
    let first = harness.navigate(1);
    harness.entered(1).await;

    // The engine cancels the superseded navigation's token, then names the next one.
    first.cancel();
    harness.disk.available.store(80, Ordering::SeqCst);
    harness.navigate(2);
    harness.entered(2).await;
    harness.disk.hold(false);
    assert_eq!(harness.space().await, (NavigationId(2), 80));

    // A request for the old navigation reads nothing.
    harness.controller.refresh(PaneId::Left, NavigationId(1));
    harness.idle(TIMING.periodic * 2).await;
    assert_eq!(harness.disk.reads(), 2);
    assert!(harness.spaces_sent().is_empty());
    harness.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn only_a_fixed_local_disk_is_read_periodically_and_only_while_visible() {
    for kind in [
        StorageKind::Network,
        StorageKind::Local,
        StorageKind::Unknown,
    ] {
        let mut harness = Harness::new(kind);
        harness.navigate(1);
        assert_eq!(harness.space().await, (NavigationId(1), 100));
        harness.idle(TIMING.periodic * 10).await;
        assert_eq!(harness.disk.reads(), 1, "{kind:?}");
        harness.shutdown().await;
    }

    let mut harness = Harness::new(StorageKind::LocalSsd);
    harness.navigate(1);
    assert_eq!(harness.space().await, (NavigationId(1), 100));
    harness
        .idle(TIMING.periodic * 3 + TIMING.periodic / 2)
        .await;
    assert_eq!(harness.disk.reads(), 4);
    // An unchanged value is not sent again.
    assert!(harness.spaces_sent().is_empty());

    harness.disk.available.store(60, Ordering::SeqCst);
    assert_eq!(harness.space().await, (NavigationId(1), 60));
    let reads = harness.disk.reads();
    harness.controller.set_visible(false);
    harness.idle(TIMING.periodic * 10).await;
    assert_eq!(harness.disk.reads(), reads);

    // A request is still served while the window is hidden.
    harness.controller.refresh(PaneId::Left, NavigationId(1));
    harness.idle(TIMING.throttle * 2).await;
    assert_eq!(harness.disk.reads(), reads + 1);
    harness.controller.set_visible(true);
    harness.idle(TIMING.periodic + TIMING.throttle).await;
    assert_eq!(harness.disk.reads(), reads + 2);
    harness.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_pane_that_leaves_native_folders_is_not_read() {
    let mut harness = Harness::new(StorageKind::LocalSsd);
    harness.navigate(1);
    assert_eq!(harness.space().await, (NavigationId(1), 100));
    harness.controller.leave(PaneId::Left);
    harness.controller.refresh(PaneId::Left, NavigationId(1));
    harness.idle(TIMING.periodic * 4).await;
    assert_eq!(harness.disk.reads(), 1);
    harness.shutdown().await;
}

/// A local disk answers from the kernel's counters at once, so a request soon after
/// a read is served soon too: the terminal request of a Delete follows the read that
/// its Running progress asked for by a few milliseconds.
#[tokio::test(start_paused = true)]
async fn a_local_disk_answers_a_request_soon_after_its_previous_read() {
    for kind in [
        StorageKind::LocalSsd,
        StorageKind::Rotational,
        StorageKind::Local,
    ] {
        let mut harness = Harness::with_timing(kind, PRODUCTION_TIMING);
        harness.navigate(1);
        assert_eq!(harness.space().await, (NavigationId(1), 100));
        let read = Instant::now();

        harness.disk.available.store(300, Ordering::SeqCst);
        harness.controller.refresh(PaneId::Left, NavigationId(1));
        assert_eq!(harness.space().await, (NavigationId(1), 300));
        let waited = read.elapsed();
        assert!(waited <= Duration::from_millis(250), "{kind:?}: {waited:?}");
        harness.shutdown().await;
    }
}

/// A filesystem can report the space a deletion frees only after the deletion
/// returned: ext4 releases a large file's blocks tens of milliseconds per GiB
/// later, more while the disk is busy. A request therefore keeps a local disk
/// followed for a while, without waiting for the periodic read.
#[tokio::test(start_paused = true)]
async fn a_request_follows_a_local_disk_until_a_deletion_released_its_space() {
    let timing = PRODUCTION_TIMING;
    let mut harness = Harness::with_timing(StorageKind::LocalSsd, timing);
    harness.navigate(1);
    assert_eq!(harness.space().await, (NavigationId(1), 100));

    // The Delete ended; the read its request causes still finds the old value.
    let requested = Instant::now();
    harness.controller.refresh(PaneId::Left, NavigationId(1));
    harness.idle(Duration::from_millis(1_200)).await;
    assert!(harness.disk.reads() >= 2, "the request was not read");
    assert!(harness.spaces_sent().is_empty());

    // The filesystem released the space more than a second later.
    let released = Instant::now();
    harness.disk.available.store(900, Ordering::SeqCst);
    assert_eq!(harness.space().await, (NavigationId(1), 900));
    let waited = released.elapsed();
    assert!(waited <= Duration::from_millis(250), "{waited:?}");

    // Once the request is `follow` old, only the periodic reads remain.
    harness
        .idle((requested + timing.follow).saturating_duration_since(Instant::now()))
        .await;
    let reads = harness.disk.reads();
    harness.idle(Duration::from_millis(2_500)).await;
    assert_eq!(harness.disk.reads(), reads);
    assert!(harness.spaces_sent().is_empty());
    harness.shutdown().await;
}
