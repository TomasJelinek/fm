use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Icons {
    calls: AtomicUsize,
    change: Option<PathBuf>,
    cancel: bool,
}
impl IconService for Icons {
    fn load_icon(&self, _: &IconKey) -> Result<Option<IconPixels>, FsError> {
        Ok(None)
    }
    fn load_executable_icon(
        &self,
        _: &Path,
        _: u16,
        cancel: &Cancellation,
    ) -> Result<Option<IconPixels>, FsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(path) = &self.change {
            std::fs::write(path, b"changed-length").unwrap();
        }
        if self.cancel {
            cancel.cancel();
        }
        Ok(Some(IconPixels::new(1, 1, vec![1, 2, 3, 255]).unwrap()))
    }
}
impl TerminalService for Icons {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        panic!("artwork never starts a process")
    }
}
impl PropertyService for Icons {
    fn show_properties(&self, _: &[PathBuf]) -> Result<bool, FsError> {
        unreachable!()
    }
}
impl LocationService for Icons {
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        unreachable!()
    }
}
impl OpenService for Icons {
    fn open_entry(&self, _: &Path) -> Result<OpenEntryResult, FsError> {
        unreachable!()
    }
}
fn capture() -> crate::archives::ArchiveCapture {
    Arc::new(|path, _, _| {
        midnight_plugin_host::CapturedFile::capture(path).map_err(|error| {
            FsError::from_io("icon capture", path, io::Error::other(error.to_string()))
        })
    })
}
fn target(path: &Path) -> CommandIconTarget {
    CommandIconTarget {
        identity: "command-icon:1:tool".into(),
        executable: path.into(),
    }
}
#[test]
fn command_icon_metadata_rejects_duplicates_and_oversized_retained_allocations() {
    let path = std::env::current_exe().unwrap();
    assert!(valid_targets(&[target(&path)]));
    assert!(!valid_targets(&vec![target(&path); MAX_USER_COMMANDS + 1]));
    assert!(!valid_targets(&[target(&path), target(&path)]));
    assert!(!valid_targets(&[target(Path::new("relative"))]));
    let mut row = target(&path);
    row.identity.reserve(4096);
    assert!(!valid_targets(&[row]));
    let mut row = target(&path);
    row.executable.reserve(MAX_USER_COMMAND_CONFIG_BYTES + 1);
    assert!(!valid_targets(&[row]));
}
#[test]
fn command_icon_capture_denial_revision_change_and_cancellation_discard_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tool");
    std::fs::write(&path, b"original").unwrap();
    let service = Icons::default();
    let deny: crate::archives::ArchiveCapture = Arc::new(|_, _, _| {
        Err(FsError::from_io(
            "icon capture",
            Path::new(""),
            io::ErrorKind::PermissionDenied.into(),
        ))
    });
    assert!(load_command(Some(&service), Some(&deny), &path, &Cancellation::default()).is_none());
    assert_eq!(service.calls.load(Ordering::SeqCst), 0);
    assert!(
        load_command(
            Some(&service),
            Some(&capture()),
            &path,
            &Cancellation::default()
        )
        .is_some()
    );
    let service = Icons {
        change: Some(path.clone()),
        ..Icons::default()
    };
    assert!(
        load_command(
            Some(&service),
            Some(&capture()),
            &path,
            &Cancellation::default()
        )
        .is_none()
    );
    assert_eq!(service.calls.load(Ordering::SeqCst), 1);
    let service = Icons {
        cancel: true,
        ..Icons::default()
    };
    assert!(
        load_command(
            Some(&service),
            Some(&capture()),
            &path,
            &Cancellation::default()
        )
        .is_none()
    );
    assert_eq!(service.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn icon_cache_forgets_the_least_recently_requested_type_instead_of_refusing_new_ones() {
    let key = |index: usize| IconKey {
        extension: Some(format!("log{index}").into()),
        ..IconKey::file()
    };
    let mut recent = RecentIcons::default();
    for index in 0..ICON_CAPACITY {
        assert_eq!(recent.admit(&key(index), &[16]), [16]);
    }
    // Rotated logs filled the cache; type 0 was used again recently.
    assert!(recent.admit(&key(0), &[16]).is_empty());
    let pdf = IconKey {
        extension: Some("pdf".into()),
        ..key(0)
    };
    assert_eq!(
        recent.admit(&pdf, &[16]),
        [16],
        "a new type still loads when the cache is full"
    );
    assert!(recent.admit(&pdf, &[16]).is_empty());
    assert!(
        recent.admit(&key(0), &[16]).is_empty(),
        "the recently used type stays cached"
    );
    assert_eq!(
        recent.admit(&key(1), &[16]),
        [16],
        "the least recently requested type was forgotten and loads again"
    );
    assert_eq!(recent.icons.len(), ICON_CAPACITY);
}

#[test]
fn recent_icons_load_only_missing_edges_and_follow_changed_edges() {
    let mut recent = RecentIcons::default();
    let (file, folder) = (IconKey::file(), IconKey::folder());
    assert_eq!(recent.admit(&file, &[20, 96]), [20, 96]);
    assert!(recent.admit(&file, &[20, 96]).is_empty());
    assert_eq!(recent.admit(&file, &[20, 96, 24]), [24]);
    assert_eq!(recent.admit(&folder, &[20]), [20]);
    assert!(recent.wants(&folder.clone().sized(20)));
    assert!(!recent.wants(&folder.clone().sized(96)));
    assert!(
        !recent.wants(&IconKey::drive().sized(20)),
        "never requested"
    );

    // Folders were requested last, so their new edges come first.
    let missing = recent.resize(|kind| match kind {
        IconKind::Folder => vec![25, 20],
        _ => vec![25],
    });
    assert_eq!(
        missing,
        [folder.clone().sized(25), file.clone().sized(25)],
        "an edge delivered before is not loaded again"
    );
    assert!(recent.wants(&folder.clone().sized(20)));
    assert!(recent.wants(&file.clone().sized(25)));
    for stale in [20, 24, 96] {
        assert!(!recent.wants(&file.clone().sized(stale)), "{stale}");
    }
    // Returning to an edge that was dropped loads it again: the frontend
    // discards artwork of edges it no longer draws.
    assert_eq!(recent.resize(|_| vec![20]), [file.sized(20)]);
}

/// Holds one load until the test releases it, also when the test fails.
#[derive(Default)]
struct LoadGate {
    entered: tokio::sync::Notify,
    released: std::sync::Mutex<bool>,
    wake: std::sync::Condvar,
}
struct Release(Arc<LoadGate>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = true;
        self.0.wake.notify_all();
    }
}

/// A provider that renders any requested edge and records what it was asked.
#[derive(Default)]
struct SizedIcons {
    keys: std::sync::Mutex<Vec<IconKey>>,
    fixed: bool,
    /// The load of this key waits at the gate.
    held: Option<(IconKey, Arc<LoadGate>)>,
}
impl IconService for SizedIcons {
    fn load_icon(&self, key: &IconKey) -> Result<Option<IconPixels>, FsError> {
        self.keys.lock().unwrap().push(key.clone());
        if let Some((held, gate)) = &self.held
            && held == key
        {
            gate.entered.notify_one();
            let mut released = gate.released.lock().unwrap();
            while !*released {
                released = gate.wake.wait(released).unwrap();
            }
        }
        let edge = u32::from(key.size);
        Ok(Some(
            IconPixels::new(edge, edge, vec![255; (edge * edge * 4) as usize]).unwrap(),
        ))
    }
    fn icon_edge(&self, drawn: u16) -> u16 {
        if self.fixed {
            IconKey::DEFAULT_EDGE
        } else {
            drawn
        }
    }
}
impl TerminalService for SizedIcons {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        unreachable!()
    }
}
impl PropertyService for SizedIcons {
    fn show_properties(&self, _: &[PathBuf]) -> Result<bool, FsError> {
        unreachable!()
    }
}
impl LocationService for SizedIcons {
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        unreachable!()
    }
}
impl OpenService for SizedIcons {
    fn open_entry(&self, _: &Path) -> Result<OpenEntryResult, FsError> {
        unreachable!()
    }
}

struct SizedWorker {
    services: Arc<SizedIcons>,
    icons: mpsc::Sender<IconKey>,
    edges: watch::Sender<IconEdges>,
    events: mpsc::Receiver<Event>,
}

fn sized_worker(fixed: bool) -> SizedWorker {
    sized_worker_with(SizedIcons {
        fixed,
        ..SizedIcons::default()
    })
}

fn sized_worker_with(services: SizedIcons) -> SizedWorker {
    let services = Arc::new(services);
    let (icons, receiver) = mpsc::channel(64);
    let (views, commands) = watch::channel(View::default());
    drop(views);
    let (edges, drawn) = watch::channel(IconEdges::default());
    let (sender, events) = mpsc::channel(64);
    tokio::spawn(worker(
        TaskLifetime::testing(services.clone()),
        Some(services.clone()),
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        receiver,
        commands,
        drawn,
    ));
    SizedWorker {
        services,
        icons,
        edges,
        events,
    }
}

impl SizedWorker {
    /// The next delivered key with the edge of its bitmap.
    async fn next(&mut self) -> (IconKey, u32) {
        let event = tokio::time::timeout(Duration::from_secs(4), self.events.recv())
            .await
            .expect("an icon is delivered");
        match event {
            Some(Event::IconLoaded { key, pixels }) => (key, pixels.width),
            other => panic!("unexpected event: {other:?}"),
        }
    }
    /// The worker handles requests in order: a not yet requested `sentinel`
    /// delivered next proves that nothing else was pending before it.
    async fn idle(&mut self, sentinel: ActionIcon) {
        self.icons.send(IconKey::action(sentinel)).await.unwrap();
        let (key, _) = self.next().await;
        assert_eq!(
            key.kind,
            IconKind::Action(sentinel),
            "an unexpected icon was delivered"
        );
    }
}

#[tokio::test]
async fn icons_are_delivered_at_each_edge_their_kind_is_drawn_at() {
    let mut worker = sized_worker(false);
    worker
        .edges
        .send(IconEdges {
            entry: 20,
            tile: 96,
            location: 26,
        })
        .unwrap();
    worker.icons.send(IconKey::file()).await.unwrap();
    assert_eq!(worker.next().await, (IconKey::file().sized(20), 20));
    assert_eq!(worker.next().await, (IconKey::file().sized(96), 96));
    // A request names the icon, whatever edge its key carries.
    worker.icons.send(IconKey::file().sized(7)).await.unwrap();
    // Action art keeps the default edge; the parent row also draws its own.
    worker
        .icons
        .send(IconKey::action(ActionIcon::Copy))
        .await
        .unwrap();
    assert_eq!(worker.next().await, (IconKey::action(ActionIcon::Copy), 32));
    let parent = IconKey::action(ActionIcon::Parent);
    worker.icons.send(parent.clone()).await.unwrap();
    for edge in [32, 20, 96] {
        assert_eq!(
            worker.next().await,
            (parent.clone().sized(edge), u32::from(edge))
        );
    }
    let home = IconKey {
        kind: IconKind::Location(LocationKind::Home),
        ..IconKey::folder()
    };
    worker.icons.send(home.clone()).await.unwrap();
    assert_eq!(worker.next().await, (home.sized(26), 26));
    worker.icons.send(IconKey::folder()).await.unwrap();
    for edge in [20, 96, 26] {
        assert_eq!(
            worker.next().await,
            (IconKey::folder().sized(edge), u32::from(edge))
        );
    }
    worker.idle(ActionIcon::Exit).await;
}

#[tokio::test]
async fn changed_edges_reload_delivered_icons_and_equal_edges_do_not() {
    let mut worker = sized_worker(false);
    let list = IconEdges {
        entry: 20,
        tile: 0,
        location: 20,
    };
    worker.edges.send(list).unwrap();
    worker.icons.send(IconKey::file()).await.unwrap();
    assert_eq!(worker.next().await, (IconKey::file().sized(20), 20));
    worker.icons.send(IconKey::folder()).await.unwrap();
    assert_eq!(worker.next().await, (IconKey::folder().sized(20), 20));

    // A tile view opens: only the tile edge is new, most recent icon first.
    worker.edges.send(IconEdges { tile: 96, ..list }).unwrap();
    assert_eq!(worker.next().await, (IconKey::folder().sized(96), 96));
    assert_eq!(worker.next().await, (IconKey::file().sized(96), 96));
    worker.idle(ActionIcon::Exit).await;
    // Sending the edges already in effect changes nothing.
    worker.edges.send(IconEdges { tile: 96, ..list }).unwrap();
    worker.idle(ActionIcon::About).await;

    // A denser display: every drawn edge changed, except that of action art.
    worker
        .edges
        .send(IconEdges {
            entry: 25,
            tile: 120,
            location: 25,
        })
        .unwrap();
    let mut reloaded = HashSet::new();
    for _ in 0..4 {
        reloaded.insert(worker.next().await);
    }
    assert_eq!(
        reloaded,
        HashSet::from([
            (IconKey::file().sized(25), 25),
            (IconKey::file().sized(120), 120),
            (IconKey::folder().sized(25), 25),
            (IconKey::folder().sized(120), 120),
        ])
    );
    worker.idle(ActionIcon::Help).await;
    let asked = worker.services.keys.lock().unwrap().clone();
    assert_eq!(
        asked.iter().collect::<HashSet<_>>().len(),
        asked.len(),
        "no icon was loaded twice at one edge: {asked:?}"
    );
}

#[tokio::test]
async fn edges_changed_while_loads_are_queued_drop_the_stale_ones_and_duplicate_none() {
    let gate = Arc::new(LoadGate::default());
    let release = Release(gate.clone());
    let rows_and_tiles = IconEdges {
        entry: 20,
        tile: 96,
        location: 26,
    };
    let rows = IconEdges {
        tile: 0,
        ..rows_and_tiles
    };
    let mut worker = sized_worker_with(SizedIcons {
        held: Some((IconKey::file().sized(20), gate.clone())),
        ..SizedIcons::default()
    });
    worker.edges.send(rows_and_tiles).unwrap();
    worker.icons.send(IconKey::file()).await.unwrap();
    // The row bitmap is loading; the tile bitmap of the same icon is queued.
    tokio::time::timeout(Duration::from_secs(4), gate.entered.notified())
        .await
        .expect("the row load started");
    // The tile view closes and opens again before the worker comes back: the
    // queued tile key is dropped, then requested anew exactly once.
    worker.edges.send(rows).unwrap();
    drop(release);
    assert_eq!(worker.next().await, (IconKey::file().sized(20), 20));
    worker.idle(ActionIcon::Exit).await;
    assert_eq!(
        *worker.services.keys.lock().unwrap(),
        [IconKey::file().sized(20), IconKey::action(ActionIcon::Exit)],
        "a tile bitmap nobody draws was loaded"
    );
    worker.edges.send(rows_and_tiles).unwrap();
    assert_eq!(worker.next().await, (IconKey::file().sized(96), 96));
    worker.idle(ActionIcon::About).await;
    let asked = worker.services.keys.lock().unwrap().clone();
    assert_eq!(
        asked.iter().collect::<HashSet<_>>().len(),
        asked.len(),
        "{asked:?}"
    );
}

#[tokio::test]
async fn a_new_request_is_served_before_the_reloads_of_an_edges_change() {
    let mut worker = sized_worker(false);
    let rows = IconEdges {
        entry: 20,
        tile: 0,
        location: 20,
    };
    worker.edges.send(rows).unwrap();
    let typed = |index: usize| IconKey {
        extension: Some(format!("type{index}").into()),
        ..IconKey::file()
    };
    for index in 0..8 {
        worker.icons.send(typed(index)).await.unwrap();
        assert_eq!(worker.next().await, (typed(index).sized(20), 20));
    }
    // Eight tile reloads are due, and a folder the user just opened lists a
    // new type: its row bitmap must not wait behind all of them.
    worker.edges.send(IconEdges { tile: 96, ..rows }).unwrap();
    worker.icons.send(typed(99)).await.unwrap();
    let mut order = Vec::new();
    for _ in 0..10 {
        order.push(worker.next().await.0);
    }
    let fresh = order
        .iter()
        .position(|key| *key == typed(99).sized(20))
        .expect("the new type is delivered");
    assert!(fresh <= 1, "delivered after {fresh} reloads: {order:?}");
    assert_eq!(
        order.iter().collect::<HashSet<_>>().len(),
        10,
        "eight tile reloads and both bitmaps of the new type, each once"
    );
    worker.idle(ActionIcon::Exit).await;
}

#[tokio::test]
async fn a_single_size_provider_is_asked_once_for_every_drawn_edge() {
    let mut worker = sized_worker(true);
    worker
        .edges
        .send(IconEdges {
            entry: 20,
            tile: 96,
            location: 26,
        })
        .unwrap();
    worker.icons.send(IconKey::folder()).await.unwrap();
    assert_eq!(worker.next().await, (IconKey::folder(), 32));
    worker.icons.send(IconKey::file()).await.unwrap();
    assert_eq!(worker.next().await, (IconKey::file(), 32));
    worker
        .edges
        .send(IconEdges {
            entry: 40,
            tile: 192,
            location: 52,
        })
        .unwrap();
    worker.idle(ActionIcon::Exit).await;
    assert_eq!(worker.services.keys.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn command_icons_progress_before_a_full_queue_of_cached_ordinary_duplicates() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tool");
    std::fs::write(&path, b"fixture").unwrap();
    let services = Arc::new(Icons::default());
    let (icons, receiver) = mpsc::channel(64);
    for _ in 0..64 {
        icons.send(IconKey::folder()).await.unwrap();
    }
    // The free slots of the icon channel each time command artwork is captured.
    let capacities = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = capacities.clone();
    let weak = icons.downgrade();
    let capture: crate::archives::ArchiveCapture = Arc::new(move |path, _, _| {
        observed
            .lock()
            .unwrap()
            .push(weak.upgrade().unwrap().capacity());
        midnight_plugin_host::CapturedFile::capture(path).map_err(|error| {
            FsError::from_io("icon capture", path, io::Error::other(error.to_string()))
        })
    });
    let (views, commands) = watch::channel(View::default());
    views
        .send(View {
            generation: 1,
            targets: vec![
                target(&path),
                CommandIconTarget {
                    identity: "command-icon:2:tool".into(),
                    ..target(&path)
                },
            ],
            cancel: Cancellation::default(),
        })
        .unwrap();
    let (sender, mut events) = mpsc::channel(4);
    let task = tokio::spawn(worker(
        TaskLifetime::testing(services.clone()),
        Some(services),
        Some(capture),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        receiver,
        commands,
        watch::channel(IconEdges::default()).1,
    ));
    tokio::time::timeout(Duration::from_secs(4), async {
        let mut loaded = 0;
        while loaded < 2 {
            if let Event::CommandIconLoaded { .. } = events
                .recv()
                .await
                .expect("worker returned before command artwork")
            {
                loaded += 1;
            }
        }
    })
    .await
    .unwrap();
    // The first folder is loaded, then the first command; the second folder is
    // already delivered and still takes exactly one turn before the second
    // command. Draining the duplicates first would leave all 64 slots free.
    assert_eq!(
        *capacities.lock().unwrap(),
        [1, 2],
        "duplicates starved visible artwork"
    );
    drop(icons);
    drop(views);
    task.await.unwrap();
}

#[tokio::test]
async fn command_icon_admission_rejects_stale_and_oversized_views_and_cancels_replaced_tokens() {
    let (sender, _events) = mpsc::channel(64);
    let mut tasks = tokio::task::JoinSet::new();
    let desktop = Desktop::testing(
        None,
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let path = std::env::current_exe().unwrap();
    desktop.command_icons(2, vec![target(&path)]);
    let active = desktop.command_icons.borrow().cancel.clone();
    desktop.command_icons(1, vec![]);
    assert_eq!(desktop.command_icons.borrow().generation, 2);
    assert!(!active.is_cancelled());
    desktop.command_icons(3, vec![target(&path), target(&path)]);
    assert_eq!(desktop.command_icons.borrow().generation, 2);
    let mut oversized = Vec::with_capacity(MAX_USER_COMMANDS + 1);
    oversized.push(target(&path));
    desktop.command_icons(3, oversized);
    assert_eq!(desktop.command_icons.borrow().generation, 2);
    assert!(
        !active.is_cancelled(),
        "refused requests leave current artwork alive"
    );
    desktop.command_icons(3, vec![]);
    assert!(active.is_cancelled());
    let current = desktop.command_icons.borrow().cancel.clone();
    drop(desktop);
    assert!(
        current.is_cancelled(),
        "controller shutdown cancels native work"
    );
}
