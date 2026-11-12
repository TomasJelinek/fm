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
    let capacity_at_capture = Arc::new(AtomicUsize::new(usize::MAX));
    let observed = capacity_at_capture.clone();
    let weak = icons.downgrade();
    let capture: crate::archives::ArchiveCapture = Arc::new(move |path, _, _| {
        observed.store(weak.upgrade().unwrap().capacity(), Ordering::SeqCst);
        midnight_plugin_host::CapturedFile::capture(path).map_err(|error| {
            FsError::from_io("icon capture", path, io::Error::other(error.to_string()))
        })
    });
    let (views, commands) = watch::channel(View::default());
    views
        .send(View {
            generation: 1,
            targets: vec![target(&path)],
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
    ));
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            match events
                .recv()
                .await
                .expect("worker returned before command artwork")
            {
                Event::CommandIconLoaded { .. } => break,
                _ => continue,
            }
        }
    })
    .await
    .unwrap();
    assert!(
        capacity_at_capture.load(Ordering::SeqCst) < 64,
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
