use midnight_core::{
    Cancellation, Command, DirectoryService, Entry, EntryMetadata, Event, FsError, NavigationId,
    PaneId, SortSpec, SpaceInfo, StorageService, Volume, VolumeService,
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
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            files,
            sender,
        )
        .run(receiver),
    );
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
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(64);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            files.clone(),
            sender,
        )
        .run(receiver),
    );
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
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(64);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            files.clone(),
            sender,
        )
        .run(receiver),
    );
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
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(64);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            files.clone(),
            sender,
        )
        .run(receiver),
    );
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

#[tokio::test]
async fn duplicate_roots_share_cache_and_periodic_refresh_replaces_expired_details() {
    let files = Arc::new(Files::new(&["same", "same"]));
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(64);
    let engine = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            files.clone(),
            sender,
        )
        .run(receiver),
    );
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
