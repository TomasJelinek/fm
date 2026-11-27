use midnight_core::{
    Cancellation, Command, DirectoryService, Entry, EntryMetadata, Event, FsError, NavigationId,
    PaneId, SortSpec, SpaceInfo, StorageService, Volume, VolumeService, WatchService,
    WatchSubscription,
};
use midnight_engine::Engine;
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, mpsc};

#[derive(Default)]
struct Gate {
    released: Mutex<bool>,
    wake: Condvar,
    entered: Notify,
}

impl Gate {
    fn wait(&self) {
        self.entered.notify_one();
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
    }

    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}

struct Release(Arc<Gate>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct Files {
    roots: Mutex<Vec<Volume>>,
    discovery_gate: Option<Arc<Gate>>,
    /// Armed by a test while it runs: the next discoveries wait on it.
    held_discovery: Mutex<Option<Arc<Gate>>>,
    metadata_gate: Option<Arc<Gate>>,
    discovery_calls: AtomicUsize,
    label_calls: AtomicUsize,
    space_calls: AtomicUsize,
}

fn volume(name: &str) -> Volume {
    Volume {
        id: name.into(),
        label: name.into(),
        root: PathBuf::from(name),
    }
}

impl Files {
    fn new(names: &[&str]) -> Self {
        Self {
            roots: Mutex::new(names.iter().map(|name| volume(name)).collect()),
            discovery_gate: None,
            held_discovery: Mutex::new(None),
            metadata_gate: None,
            discovery_calls: AtomicUsize::new(0),
            label_calls: AtomicUsize::new(0),
            space_calls: AtomicUsize::new(0),
        }
    }
}

impl DirectoryService for Files {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        Ok(())
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        unreachable!("empty directory")
    }
}

impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        self.discovery_calls.fetch_add(1, Ordering::SeqCst);
        let roots = self.roots.lock().unwrap().clone();
        if let Some(gate) = &self.discovery_gate {
            gate.wait();
        }
        let held = self.held_discovery.lock().unwrap().clone();
        if let Some(gate) = held {
            gate.wait();
        }
        Ok(roots)
    }

    fn volume_label(&self, path: &Path) -> Result<Option<OsString>, FsError> {
        self.label_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.metadata_gate {
            gate.wait();
        }
        if path == Path::new("label-error") {
            Err(FsError::from_io(
                "label",
                path,
                io::ErrorKind::PermissionDenied.into(),
            ))
        } else {
            Ok(Some(OsString::from("Žluťoučký 磁碟 🦀")))
        }
    }
}

impl StorageService for Files {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        self.space_calls.fetch_add(1, Ordering::SeqCst);
        if path == Path::new("space-error") {
            Err(FsError::from_io(
                "space",
                path,
                io::ErrorKind::NotFound.into(),
            ))
        } else {
            Ok(SpaceInfo {
                available: 40,
                total: 100,
            })
        }
    }
}

async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn until(events: &mut mpsc::Receiver<Event>, condition: impl Fn(&Event) -> bool) -> Event {
    loop {
        let event = next(events).await;
        if condition(&event) {
            return event;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metadata_errors_are_independent_and_preserve_unicode_names() {
    let files = Arc::new(Files::new(&["label-error", "space-error", "good"]));

    let (sender, mut events) = mpsc::channel(32);
    let mut command_engine_1 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        files,
        sender,
    );
    let (commands, receiver) = command_engine_1.command_channel(8).unwrap();
    let engine = tokio::spawn(command_engine_1.run(receiver));
    commands.send(Command::ListVolumes).await.unwrap();
    until(&mut events, |event| {
        assert!(!matches!(event, Event::VolumeDetailsUpdated { .. }));
        matches!(event, Event::VolumesUpdated(_))
    })
    .await;
    let mut received = 0;
    while received < 3 {
        if let Event::VolumeDetailsUpdated {
            volume_id, details, ..
        } = next(&mut events).await
        {
            match volume_id.as_str() {
                "label-error" => {
                    assert_eq!(
                        details.label.unwrap_err().kind,
                        io::ErrorKind::PermissionDenied
                    );
                    assert_eq!(details.space.unwrap().available, 40);
                }
                "space-error" => {
                    assert_eq!(
                        details.label.unwrap(),
                        Some(OsString::from("Žluťoučký 磁碟 🦀"))
                    );
                    assert_eq!(details.space.unwrap_err().kind, io::ErrorKind::NotFound);
                }
                "good" => {
                    assert!(details.label.is_ok());
                    assert!(details.space.is_ok());
                }
                _ => panic!("unexpected volume"),
            }
            received += 1;
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_details_do_not_block_discovery_navigation_or_exceed_two_workers() {
    let gate = Arc::new(Gate::default());
    let _release = Release(gate.clone());
    let mut files = Files::new(&["a", "b", "c", "d"]);
    files.metadata_gate = Some(gate.clone());
    let files = Arc::new(files);

    let (sender, mut events) = mpsc::channel(64);
    let mut command_engine_2 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        files.clone(),
        sender,
    );
    let (commands, receiver) = command_engine_2.command_channel(8).unwrap();
    let engine = tokio::spawn(command_engine_2.run(receiver));
    commands.send(Command::ListVolumes).await.unwrap();
    until(&mut events, |event| {
        matches!(event, Event::VolumesUpdated(_))
    })
    .await;
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    for _ in 0..40 {
        commands.send(Command::ListVolumes).await.unwrap();
    }
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: "fast".into(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    let (mut listing, mut discovery) = (false, false);
    while !listing || !discovery {
        match next(&mut events).await {
            Event::ListingFinished { .. } => listing = true,
            Event::VolumesUpdated(_) => discovery = true,
            _ => {}
        }
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(files.label_calls.load(Ordering::SeqCst) <= 2);
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_flood_coalesces_and_discards_superseded_discovery() {
    let gate = Arc::new(Gate::default());
    let _release = Release(gate.clone());
    let mut files = Files::new(&["old"]);
    files.discovery_gate = Some(gate.clone());
    let files = Arc::new(files);

    let (sender, mut events) = mpsc::channel(64);
    let mut command_engine_3 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        files.clone(),
        sender,
    );
    let (commands, receiver) = command_engine_3.command_channel(8).unwrap();
    let engine = tokio::spawn(command_engine_3.run(receiver));
    commands.send(Command::ListVolumes).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    *files.roots.lock().unwrap() = vec![volume("new")];
    for _ in 0..100 {
        commands.send(Command::ListVolumes).await.unwrap();
    }
    commands
        .send(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: "fast".into(),
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    until(&mut events, |event| {
        matches!(event, Event::ListingFinished { .. })
    })
    .await;
    assert_eq!(files.discovery_calls.load(Ordering::SeqCst), 1);
    gate.release();
    if let Event::VolumesUpdated(volumes) = until(&mut events, |event| {
        matches!(event, Event::VolumesUpdated(_))
    })
    .await
    {
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].id, "new");
    }
    assert_eq!(files.discovery_calls.load(Ordering::SeqCst), 2);
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replaced_raw_root_rejects_old_metadata_for_same_volume_id() {
    let gate = Arc::new(Gate::default());
    let _release = Release(gate.clone());
    let mut files = Files::new(&["old"]);
    files.metadata_gate = Some(gate.clone());
    let files = Arc::new(files);

    let (sender, mut events) = mpsc::channel(64);
    let mut command_engine_4 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        files.clone(),
        sender,
    );
    let (commands, receiver) = command_engine_4.command_channel(8).unwrap();
    let engine = tokio::spawn(command_engine_4.run(receiver));
    commands.send(Command::ListVolumes).await.unwrap();
    until(&mut events, |event| {
        matches!(event, Event::VolumesUpdated(_))
    })
    .await;
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    files.roots.lock().unwrap()[0].root = PathBuf::from("new");
    commands.send(Command::ListVolumes).await.unwrap();
    until(&mut events, |event| {
        matches!(event, Event::VolumesUpdated(_))
    })
    .await;
    gate.release();
    if let Event::VolumeDetailsUpdated {
        root, volume_id, ..
    } = until(&mut events, |event| {
        matches!(event, Event::VolumeDetailsUpdated { .. })
    })
    .await
    {
        assert_eq!(volume_id, "old");
        assert_eq!(root, PathBuf::from("new"));
    }
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

type Hint = Arc<dyn Fn() + Send + Sync>;

/// A native drive watch whose hints the test sends by hand.
#[derive(Default)]
struct Drives {
    hint: Mutex<Option<Hint>>,
    registrations: AtomicUsize,
    dropped: Arc<AtomicUsize>,
}

struct DrivesSubscription(Arc<AtomicUsize>);
impl WatchSubscription for DrivesSubscription {}
impl Drop for DrivesSubscription {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl WatchService for Drives {
    fn watch_directory(
        &self,
        path: &Path,
        _: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        Err(FsError::from_io(
            "watch",
            path,
            io::ErrorKind::Unsupported.into(),
        ))
    }

    fn watch_volumes(&self, changed: Hint) -> Result<Box<dyn WatchSubscription>, FsError> {
        self.registrations.fetch_add(1, Ordering::SeqCst);
        *self.hint.lock().unwrap() = Some(changed);
        Ok(Box::new(DrivesSubscription(self.dropped.clone())))
    }
}

impl Drives {
    async fn registered(&self) -> Hint {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(hint) = self.hint.lock().unwrap().clone() {
                return hint;
            }
            assert!(std::time::Instant::now() < deadline, "no drive watch");
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}

fn watched_engine(
    files: Arc<Files>,
    drives: Arc<Drives>,
) -> (
    midnight_engine::CommandSender,
    mpsc::Receiver<Event>,
    tokio::task::JoinHandle<()>,
) {
    let (sender, events) = mpsc::channel(256);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        files,
        sender,
    );
    engine.configure_watch().unwrap().install(drives);
    let (commands, receiver) = engine.command_channel(8).unwrap();
    (commands, events, tokio::spawn(engine.run(receiver)))
}

fn names(volumes: &[Volume]) -> Vec<&str> {
    volumes.iter().map(|volume| volume.id.as_str()).collect()
}

/// Wait for every event the engine sends within `quiet` of the previous one.
async fn drain(events: &mut mpsc::Receiver<Event>, quiet: Duration) -> Vec<Event> {
    let mut drained = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(quiet, events.recv()).await {
        drained.push(event);
    }
    drained
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_drive_hint_lists_the_new_drive_at_once_before_its_blocked_details() {
    let gate = Arc::new(Gate::default());
    let _release = Release(gate.clone());
    let mut files = Files::new(&["C:"]);
    files.metadata_gate = Some(gate.clone());
    let files = Arc::new(files);
    let drives = Arc::new(Drives::default());
    let (commands, mut events, engine) = watched_engine(files.clone(), drives.clone());
    commands.send(Command::ListVolumes).await.unwrap();
    until(&mut events, |event| {
        matches!(event, Event::VolumesUpdated(_))
    })
    .await;
    let hint = drives.registered().await;
    // The label read of C: holds one of the two details workers.
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    files.roots.lock().unwrap().push(volume("E:"));
    let hinted = std::time::Instant::now();
    hint();
    let Event::VolumesUpdated(volumes) = until(
        &mut events,
        |event| matches!(event, Event::VolumesUpdated(volumes) if volumes.len() == 2),
    )
    .await
    else {
        unreachable!()
    };
    let latency = hinted.elapsed();
    assert_eq!(names(&volumes), ["C:", "E:"]);
    assert!(
        latency < Duration::from_millis(200),
        "listed after {latency:?}"
    );
    files.roots.lock().unwrap().pop();
    hint();
    let Event::VolumesUpdated(volumes) = until(&mut events, |event| {
        matches!(event, Event::VolumesUpdated(_))
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(names(&volumes), ["C:"]);
    assert_eq!(drives.registrations.load(Ordering::SeqCst), 1);
    gate.release();
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
    assert_eq!(drives.dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unchanged_hints_publish_nothing_and_a_burst_coalesces() {
    let files = Arc::new(Files::new(&["C:", "D:"]));
    let drives = Arc::new(Drives::default());
    let (commands, mut events, engine) = watched_engine(files.clone(), drives.clone());
    commands.send(Command::ListVolumes).await.unwrap();
    let hint = drives.registered().await;
    // The first list, its details and the check after registration.
    let settled = drain(&mut events, Duration::from_millis(300)).await;
    assert_eq!(
        settled
            .iter()
            .filter(|event| matches!(event, Event::VolumesUpdated(_)))
            .count(),
        1
    );
    let before = files.discovery_calls.load(Ordering::SeqCst);
    // The burst arrives while the discovery that its first hint starts is held, so a
    // runner that stalls between hints cannot spread it over several discoveries
    // (CI once counted 3): one discovery and at most one more for the hints after it.
    let held = Arc::new(Gate::default());
    let _release = Release(held.clone());
    *files.held_discovery.lock().unwrap() = Some(held.clone());
    hint();
    tokio::time::timeout(Duration::from_secs(3), held.entered.notified())
        .await
        .expect("the burst's first discovery starts");
    for _ in 1..500 {
        hint();
    }
    *files.held_discovery.lock().unwrap() = None;
    held.release();
    let quiet = drain(&mut events, Duration::from_millis(300)).await;
    assert!(
        quiet
            .iter()
            .all(|event| !matches!(event, Event::VolumesUpdated(_))),
        "an unchanged list was published again"
    );
    let calls = files.discovery_calls.load(Ordering::SeqCst) - before;
    assert!(
        (1..=2).contains(&calls),
        "{calls} discoveries for one burst"
    );
    assert_eq!(files.label_calls.load(Ordering::SeqCst), 2);
    // An explicit request still answers with the unchanged list.
    commands.send(Command::ListVolumes).await.unwrap();
    until(&mut events, |event| {
        matches!(event, Event::VolumesUpdated(_))
    })
    .await;
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hint_reads_failed_details_again() {
    let files = Arc::new(Files::new(&["space-error", "good"]));
    let drives = Arc::new(Drives::default());
    let (commands, mut events, engine) = watched_engine(files.clone(), drives.clone());
    commands.send(Command::ListVolumes).await.unwrap();
    let hint = drives.registered().await;
    drain(&mut events, Duration::from_millis(300)).await;
    assert_eq!(files.space_calls.load(Ordering::SeqCst), 2);
    hint();
    let Event::VolumeDetailsUpdated { volume_id, .. } = until(&mut events, |event| {
        matches!(event, Event::VolumeDetailsUpdated { .. })
    })
    .await
    else {
        unreachable!()
    };
    // Only the drive whose details failed (an empty card reader, say) is read again.
    assert_eq!(volume_id, "space-error");
    drain(&mut events, Duration::from_millis(100)).await;
    assert_eq!(files.space_calls.load(Ordering::SeqCst), 3);
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}

#[tokio::test]
async fn duplicate_roots_share_cache_and_periodic_refresh_replaces_expired_details() {
    let files = Arc::new(Files::new(&["same", "same"]));

    let (sender, mut events) = mpsc::channel(64);
    let mut command_engine_5 = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        files.clone(),
        sender,
    );
    let (commands, receiver) = command_engine_5.command_channel(8).unwrap();
    let engine = tokio::spawn(command_engine_5.run(receiver));
    commands.send(Command::ListVolumes).await.unwrap();
    until(&mut events, |event| {
        matches!(event, Event::VolumeDetailsUpdated { .. })
    })
    .await;
    commands.send(Command::ListVolumes).await.unwrap();
    until(&mut events, |event| {
        matches!(event, Event::VolumesUpdated(_))
    })
    .await;
    until(&mut events, |event| {
        matches!(event, Event::VolumeDetailsUpdated { .. })
    })
    .await;
    assert_eq!(files.label_calls.load(Ordering::SeqCst), 1);
    assert_eq!(files.space_calls.load(Ordering::SeqCst), 1);
    tokio::time::pause();
    let refresh_due = tokio::time::Instant::now() + Duration::from_secs(45);
    tokio::time::advance(Duration::from_secs(44)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(files.label_calls.load(Ordering::SeqCst), 1);
    // Tokio rounds deadlines up to a millisecond tick. Advance explicitly:
    // persistent archive-size blocking lanes inhibit paused-clock auto-advance.
    // Stop at this refresh tick so a missed 45s cache refresh still fails below.
    tokio::time::advance(
        (refresh_due + Duration::from_millis(1))
            .saturating_duration_since(tokio::time::Instant::now()),
    )
    .await;
    assert!(tokio::time::Instant::now() <= refresh_due + Duration::from_millis(1));
    // Keep the runtime runnable until real native workers finish. An event
    // receive with a timeout could auto-advance paused time to the next 45s
    // discovery tick and conceal a missed per-cache refresh deadline.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if files.space_calls.load(Ordering::SeqCst) >= 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(files.label_calls.load(Ordering::SeqCst), 2);
    assert_eq!(files.space_calls.load(Ordering::SeqCst), 2);
    loop {
        if let Event::VolumeDetailsUpdated { .. } = next(&mut events).await
            && files.label_calls.load(Ordering::SeqCst) >= 2
        {
            break;
        }
    }
    assert_eq!(files.label_calls.load(Ordering::SeqCst), 2);
    assert_eq!(files.space_calls.load(Ordering::SeqCst), 2);
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}
