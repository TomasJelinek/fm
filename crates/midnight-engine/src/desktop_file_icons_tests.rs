use super::*;
use std::sync::{Condvar, Mutex};

/// Holds the load of one path until the test releases it, also when it fails.
#[derive(Default)]
struct Gate {
    entered: tokio::sync::Notify,
    released: Mutex<bool>,
    wake: Condvar,
}

struct Release(Arc<Gate>);

impl Drop for Release {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = true;
        self.0.wake.notify_all();
    }
}

/// A provider that draws a file's artwork at any edge and records each request.
#[derive(Default)]
struct FileIcons {
    loaded: Mutex<Vec<(PathBuf, u16)>>,
    held: Option<(PathBuf, Arc<Gate>)>,
    /// Rewrites the file while its artwork is read.
    change: bool,
    /// Requests cancellation while its artwork is read.
    cancel: bool,
    /// A program without artwork of its own.
    plain: Option<PathBuf>,
}

impl IconService for FileIcons {
    fn load_icon(&self, key: &IconKey) -> Result<Option<IconPixels>, FsError> {
        let edge = u32::from(key.size);
        Ok(Some(
            IconPixels::new(edge, edge, vec![255; (edge * edge * 4) as usize]).unwrap(),
        ))
    }

    fn load_file_icon(
        &self,
        path: &Path,
        edge: u16,
        cancel: &Cancellation,
    ) -> Result<Option<IconPixels>, FsError> {
        self.loaded.lock().unwrap().push((path.to_owned(), edge));
        if let Some((held, gate)) = &self.held
            && held == path
        {
            gate.entered.notify_one();
            let mut released = gate.released.lock().unwrap();
            while !*released {
                released = gate.wake.wait(released).unwrap();
            }
        }
        if self.change {
            std::fs::write(path, b"a longer, rewritten program").unwrap();
        }
        if self.cancel {
            cancel.cancel();
        }
        if self.plain.as_deref() == Some(path) {
            return Ok(None);
        }
        let edge = u32::from(edge);
        Ok(Some(
            IconPixels::new(edge, edge, vec![200; (edge * edge * 4) as usize]).unwrap(),
        ))
    }
}

impl TerminalService for FileIcons {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        panic!("artwork never starts a process")
    }
}

impl PropertyService for FileIcons {
    fn show_properties(&self, _: &[PathBuf]) -> Result<bool, FsError> {
        unreachable!()
    }
}

impl LocationService for FileIcons {
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        unreachable!()
    }
}

impl OpenService for FileIcons {
    fn open_entry(&self, _: &Path) -> Result<OpenEntryResult, FsError> {
        unreachable!()
    }
}

/// An event without its pixels, which would fill a failure message.
fn described(event: Option<&Event>) -> String {
    match event {
        Some(Event::FileIconLoaded {
            generation,
            identity,
            edge,
            ..
        }) => format!("file artwork {identity:?} of view {generation} at {edge} px"),
        Some(Event::IconLoaded { key, .. }) => format!("type icon {key:?}"),
        Some(_) => "another event".into(),
        None => "the end of the events".into(),
    }
}

fn capture() -> crate::archives::ArchiveCapture {
    Arc::new(|path, _, _| {
        midnight_plugin_host::CapturedFile::capture(path).map_err(|error| {
            FsError::from_io("icon capture", path, io::Error::other(error.to_string()))
        })
    })
}

fn target(path: &Path, identity: &str, edge: u16) -> FileIconTarget {
    FileIconTarget {
        identity: identity.into(),
        path: path.into(),
        edge,
    }
}

/// Programs in a temporary folder.
fn programs(count: usize) -> (tempfile::TempDir, Vec<PathBuf>) {
    let directory = tempfile::tempdir().unwrap();
    let paths = (0..count)
        .map(|index| {
            let path = directory.path().join(format!("program {index}.exe"));
            std::fs::write(&path, b"fixture program").unwrap();
            path
        })
        .collect();
    (directory, paths)
}

#[test]
fn file_icon_views_accept_only_bounded_distinct_drawn_targets() {
    let (_directory, paths) = programs(1);
    let path = &paths[0];
    assert!(valid_targets(&[target(path, "0:1:entry:7", 216)]));
    assert!(valid_targets(&[]));
    assert!(!valid_targets(&vec![
        target(path, "same", 96);
        MAX_FILE_ICON_TARGETS + 1
    ]));
    let many: Vec<_> = (0..=MAX_FILE_ICON_TARGETS)
        .map(|index| target(path, &format!("tile {index}"), 96))
        .collect();
    assert!(!valid_targets(&many), "more tiles than the request budget");
    assert!(valid_targets(&many[..MAX_FILE_ICON_TARGETS]));
    assert!(!valid_targets(&[
        target(path, "twice", 96),
        target(path, "twice", 128)
    ]));
    for edge in [0, MAX_ICON_EDGE + 1] {
        assert!(!valid_targets(&[target(path, "edge", edge)]), "{edge}");
    }
    assert!(!valid_targets(&[target(
        Path::new("relative.exe"),
        "relative",
        96
    )]));
    assert!(!valid_targets(&[target(path, "", 96)]));
    let mut oversized = target(path, "identity", 96);
    oversized.identity.reserve(4096);
    assert!(!valid_targets(&[oversized]));
    let mut nul = path.as_os_str().to_owned();
    nul.push("\0.exe");
    assert!(!valid_targets(&[target(Path::new(&nul), "nul", 96)]));
}

#[test]
fn file_artwork_needs_a_capture_and_an_unchanged_file() {
    let (_directory, paths) = programs(1);
    let path = &paths[0];
    let cancel = Cancellation::default();
    let service = FileIcons::default();
    let pixels = load(Some(&service), Some(&capture()), path, 216, &cancel).unwrap();
    assert_eq!((pixels.width, pixels.height), (216, 216));

    // Refused capture: the provider is never asked, there is no direct read.
    let service = FileIcons::default();
    let deny: crate::archives::ArchiveCapture = Arc::new(|_, _, _| {
        Err(FsError::from_io(
            "icon capture",
            Path::new(""),
            io::ErrorKind::PermissionDenied.into(),
        ))
    });
    assert!(load(Some(&service), Some(&deny), path, 216, &cancel).is_none());
    assert!(load(Some(&service), None, path, 216, &cancel).is_none());
    assert!(service.loaded.lock().unwrap().is_empty());

    // A file rewritten while its artwork is read yields nothing.
    let service = FileIcons {
        change: true,
        ..FileIcons::default()
    };
    assert!(load(Some(&service), Some(&capture()), path, 216, &cancel).is_none());
    assert_eq!(service.loaded.lock().unwrap().len(), 1);

    // Cancelled before or during the read.
    let service = FileIcons {
        cancel: true,
        ..FileIcons::default()
    };
    assert!(
        load(
            Some(&service),
            Some(&capture()),
            path,
            96,
            &Cancellation::default()
        )
        .is_none()
    );
    let cancelled = Cancellation::default();
    cancelled.cancel();
    let service = FileIcons::default();
    assert!(load(Some(&service), Some(&capture()), path, 96, &cancelled).is_none());
    assert!(service.loaded.lock().unwrap().is_empty());
}

#[test]
fn artwork_larger_than_its_tile_is_refused() {
    struct Oversized;
    impl IconService for Oversized {
        fn load_icon(&self, _: &IconKey) -> Result<Option<IconPixels>, FsError> {
            Ok(None)
        }
        fn load_file_icon(
            &self,
            _: &Path,
            edge: u16,
            _: &Cancellation,
        ) -> Result<Option<IconPixels>, FsError> {
            let edge = u32::from(edge) + 1;
            Ok(IconPixels::new(edge, edge, vec![255; (edge * edge * 4) as usize]).ok())
        }
    }
    impl TerminalService for Oversized {
        fn shell_flavor(&self) -> ShellFlavor {
            ShellFlavor::Posix
        }
        fn launch_terminal(
            &self,
            _: &TerminalRequest,
        ) -> Result<Box<dyn TerminalProcess>, FsError> {
            unreachable!()
        }
    }
    impl PropertyService for Oversized {
        fn show_properties(&self, _: &[PathBuf]) -> Result<bool, FsError> {
            unreachable!()
        }
    }
    impl LocationService for Oversized {
        fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
            unreachable!()
        }
    }
    impl OpenService for Oversized {
        fn open_entry(&self, _: &Path) -> Result<OpenEntryResult, FsError> {
            unreachable!()
        }
    }
    let (_directory, paths) = programs(1);
    let cancel = Cancellation::default();
    assert!(load(Some(&Oversized), Some(&capture()), &paths[0], 96, &cancel).is_none());
}

struct Worker {
    services: Arc<FileIcons>,
    views: watch::Sender<View>,
    events: mpsc::Receiver<Event>,
    icons: mpsc::Sender<IconKey>,
}

fn worker(services: FileIcons) -> Worker {
    let services = Arc::new(services);
    let (icons, receiver) = mpsc::channel(64);
    let (views, files) = watch::channel(View::default());
    let (sender, events) = mpsc::channel(64);
    tokio::spawn(super::super::command_icons::worker(
        TaskLifetime::testing(services.clone()),
        Some(services.clone()),
        Some(capture()),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        super::super::command_icons::Inputs {
            icons: receiver,
            commands: watch::channel(super::super::command_icons::View::default()).1,
            files,
            edges: watch::channel(IconEdges::default()).1,
        },
    ));
    Worker {
        services,
        views,
        events,
        icons,
    }
}

impl Worker {
    fn show(&self, generation: u64, targets: Vec<FileIconTarget>) {
        self.views.send_modify(|view| {
            view.cancel.cancel();
            *view = View {
                generation,
                targets,
                cancel: Cancellation::default(),
            };
        });
    }

    /// The next delivered file artwork: generation, identity and edge.
    async fn next(&mut self) -> (u64, String, u16) {
        let event = tokio::time::timeout(Duration::from_secs(4), self.events.recv())
            .await
            .expect("file artwork is delivered");
        match event {
            Some(Event::FileIconLoaded {
                generation,
                identity,
                edge,
                pixels,
            }) => {
                assert_eq!((pixels.width, pixels.height), (edge.into(), edge.into()));
                (generation, identity, edge)
            }
            other => panic!("unexpected event: {}", described(other.as_ref())),
        }
    }
}

#[tokio::test]
async fn visible_tiles_are_delivered_in_order_at_their_edges() {
    let (_directory, paths) = programs(3);
    let mut worker = worker(FileIcons::default());
    worker.show(
        1,
        vec![
            target(&paths[0], "a", 216),
            target(&paths[1], "b", 216),
            target(&paths[2], "c", 96),
        ],
    );
    assert_eq!(worker.next().await, (1, "a".into(), 216));
    assert_eq!(worker.next().await, (1, "b".into(), 216));
    assert_eq!(worker.next().await, (1, "c".into(), 96));
    assert_eq!(
        *worker.services.loaded.lock().unwrap(),
        [
            (paths[0].clone(), 216),
            (paths[1].clone(), 216),
            (paths[2].clone(), 96)
        ]
    );
}

#[tokio::test]
async fn a_scrolled_view_cancels_the_load_in_progress_and_drops_the_queued_tiles() {
    let (_directory, paths) = programs(4);
    let gate = Arc::new(Gate::default());
    let release = Release(gate.clone());
    let mut worker = worker(FileIcons {
        held: Some((paths[0].clone(), gate.clone())),
        ..FileIcons::default()
    });
    worker.show(
        1,
        vec![
            target(&paths[0], "a", 216),
            target(&paths[1], "b", 216),
            target(&paths[2], "c", 216),
        ],
    );
    tokio::time::timeout(Duration::from_secs(4), gate.entered.notified())
        .await
        .expect("the first tile started loading");
    // Scrolled: the first three tiles left the view while the first was loading.
    worker.show(2, vec![target(&paths[3], "d", 216)]);
    drop(release);
    assert_eq!(
        worker.next().await,
        (2, "d".into(), 216),
        "nothing of the scrolled-away view was delivered"
    );
    // File artwork and type icons take turns: a tile still queued from the old view
    // would be loaded between these two type icons.
    for extension in ["one", "two"] {
        let key = IconKey {
            extension: Some(extension.into()),
            ..IconKey::file()
        };
        worker.icons.send(key).await.unwrap();
    }
    for _ in 0..2 {
        let event = tokio::time::timeout(Duration::from_secs(4), worker.events.recv())
            .await
            .expect("the type icons are delivered");
        assert!(
            matches!(event, Some(Event::IconLoaded { .. })),
            "a tile of the scrolled-away view was loaded: {}",
            described(event.as_ref())
        );
    }
    assert_eq!(
        *worker.services.loaded.lock().unwrap(),
        [(paths[0].clone(), 216), (paths[3].clone(), 216)],
        "the queued tiles of the old view were never loaded"
    );
}

#[tokio::test]
async fn file_artwork_takes_turns_with_type_icons() {
    let (_directory, paths) = programs(3);
    let services = Arc::new(FileIcons::default());
    let (icons, receiver) = mpsc::channel(64);
    let (views, files) = watch::channel(View::default());
    views.send_modify(|view| {
        *view = View {
            generation: 1,
            targets: paths
                .iter()
                .enumerate()
                .map(|(index, path)| target(path, &format!("tile {index}"), 96))
                .collect(),
            cancel: Cancellation::default(),
        }
    });
    let typed = |index: usize| IconKey {
        extension: Some(format!("type{index}").into()),
        ..IconKey::file()
    };
    for index in 0..3 {
        icons.send(typed(index)).await.unwrap();
    }
    let (sender, mut events) = mpsc::channel(64);
    tokio::spawn(super::super::command_icons::worker(
        TaskLifetime::testing(services.clone()),
        Some(services.clone()),
        Some(capture()),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        super::super::command_icons::Inputs {
            icons: receiver,
            commands: watch::channel(super::super::command_icons::View::default()).1,
            files,
            edges: watch::channel(IconEdges::default()).1,
        },
    ));
    let mut order = Vec::new();
    while order.len() < 6 {
        let event = tokio::time::timeout(Duration::from_secs(4), events.recv())
            .await
            .expect("artwork is delivered")
            .expect("the worker runs");
        order.push(match event {
            Event::IconLoaded { .. } => 'T',
            Event::FileIconLoaded { .. } => 'F',
            other => panic!("unexpected event: {}", described(Some(&other))),
        });
    }
    let order: String = order.into_iter().collect();
    assert!(
        !order.contains("TT") && !order.contains("FF"),
        "type icons and file artwork alternate: {order}"
    );
    drop((icons, views));
}

#[tokio::test]
async fn a_program_without_artwork_is_read_once_per_listed_revision() {
    let (_directory, paths) = programs(2);
    let mut worker = worker(FileIcons {
        plain: Some(paths[0].clone()),
        ..FileIcons::default()
    });
    worker.show(
        1,
        vec![target(&paths[0], "plain", 216), target(&paths[1], "b", 216)],
    );
    assert_eq!(worker.next().await, (1, "b".into(), 216));
    // Scrolled away and back: the plain program is not read again.
    worker.show(
        2,
        vec![
            target(&paths[0], "plain", 216),
            target(&paths[1], "b again", 216),
        ],
    );
    assert_eq!(worker.next().await, (2, "b again".into(), 216));
    // Rewritten, it is listed under another identity and read again.
    worker.show(
        3,
        vec![
            target(&paths[0], "plain, rewritten", 216),
            target(&paths[1], "b once more", 216),
        ],
    );
    assert_eq!(worker.next().await, (3, "b once more".into(), 216));
    let loaded = worker.services.loaded.lock().unwrap().clone();
    let plain = loaded.iter().filter(|(path, _)| *path == paths[0]).count();
    assert_eq!(plain, 2, "{loaded:?}");
}

#[test]
fn programs_without_artwork_are_remembered_within_a_bound() {
    let mut absent = Absent::default();
    for index in 0..MAX_ABSENT + 10 {
        absent.insert(format!("tile {index}"));
    }
    absent.insert("tile 20".into());
    assert!(!absent.contains("tile 0"), "the oldest is forgotten first");
    assert!(!absent.contains("tile 9"));
    assert!(absent.contains("tile 10"));
    assert!(absent.contains(&format!("tile {}", MAX_ABSENT + 9)));
    assert_eq!(absent.order.len(), MAX_ABSENT);
    assert_eq!(absent.identities.len(), MAX_ABSENT);
}
