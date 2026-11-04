use fm_core::*;
use fm_engine::Engine;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use tokio::sync::{Notify, mpsc};

#[derive(Default)]
struct Services {
    launches: Mutex<Vec<TerminalRequest>>,
    properties_entered: Notify,
    release_properties: (Mutex<bool>, Condvar),
    icon_keys: Mutex<Vec<IconKey>>,
    opened: Mutex<Vec<PathBuf>>,
    open_entered: Notify,
    release_open: (Mutex<bool>, Condvar),
}

struct ReleaseOpen(Arc<Services>);
impl Drop for ReleaseOpen {
    fn drop(&mut self) {
        *self
            .0
            .release_open
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.0.release_open.1.notify_all();
    }
}

impl OpenService for Services {
    fn open_entry(&self, path: &Path) -> Result<OpenEntryResult, FsError> {
        self.opened.lock().unwrap().push(path.into());
        if path == Path::new("blocked-open") {
            self.open_entered.notify_one();
            let (lock, condition) = &self.release_open;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = condition.wait(released).unwrap();
            }
        }
        if path == Path::new("missing-open") {
            return Err(FsError::from_io(
                "open entry",
                path,
                io::ErrorKind::NotFound.into(),
            ));
        }
        if path == Path::new("linked-folder") {
            return Ok(OpenEntryResult::Directory);
        }
        if path == Path::new("helper-error") || path == Path::new("helper-success") {
            // The trusted test harness is an opener stand-in; no desktop app or
            // user file is opened. Invalid harness flags guarantee nonzero exit.
            let argument = if path == Path::new("helper-error") {
                "--__fm_invalid_probe_option"
            } else {
                "--list"
            };
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .arg(argument)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            return Ok(OpenEntryResult::File {
                process: Some(Box::new(child)),
            });
        }
        Ok(OpenEntryResult::File { process: None })
    }
}

impl DirectoryService for Services {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        for i in 0..150 {
            if !emit(Entry::new(
                EntryId(i),
                path.join(format!("文档-{i}.txt")),
                EntryKind::File,
            )) {
                break;
            }
        }
        Ok(())
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        Ok(EntryMetadata {
            size: Some(17),
            modified: None,
        })
    }
}
impl VolumeService for Services {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for Services {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            available: 4,
            total: 8,
        })
    }
}
impl IconService for Services {
    fn load_icon(&self, key: &IconKey) -> Result<Option<IconPixels>, FsError> {
        self.icon_keys.lock().unwrap().push(key.clone());
        Ok(matches!(key.kind, IconKind::Location(_))
            .then(|| IconPixels::new(1, 1, vec![12, 34, 56, 255]).unwrap()))
    }
}
impl LocationService for Services {
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        let mut locations = vec![NavigationLocation {
            id: "home".into(),
            label: "Home".into(),
            path: PathBuf::from("文档/🏠"),
            kind: LocationKind::Home,
            drive_letter: None,
        }];
        for kind in [
            LocationKind::Desktop,
            LocationKind::Documents,
            LocationKind::Downloads,
        ] {
            locations.push(NavigationLocation {
                id: format!("{kind:?}"),
                label: format!("{kind:?}"),
                path: PathBuf::from("文档/🏠"),
                kind,
                drive_letter: None,
            });
        }
        for letter in ['C', 'D'] {
            locations.push(NavigationLocation {
                id: format!("drive:{letter}"),
                label: letter.to_string(),
                path: PathBuf::from(format!("{letter}:\\")),
                kind: LocationKind::Volume,
                drive_letter: Some(letter),
            });
        }
        Ok(locations)
    }
}
impl TerminalService for Services {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::PowerShell
    }
    fn launch_terminal(
        &self,
        request: &TerminalRequest,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        self.launches.lock().unwrap().push(request.clone());
        Err(FsError::from_io(
            "launch terminal",
            &request.working_directory,
            io::Error::from(io::ErrorKind::NotFound),
        ))
    }
}
impl PropertyService for Services {
    fn show_properties(&self, _: &[PathBuf]) -> Result<bool, FsError> {
        self.properties_entered.notify_one();
        let (lock, condition) = &self.release_properties;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = condition.wait(released).unwrap();
        }
        Ok(false)
    }
}
async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(4), events.recv())
        .await
        .unwrap()
        .unwrap()
}
fn navigation() -> Command {
    Command::Navigate {
        pane: PaneId::Right,
        navigation: NavigationId(1),
        path: PathBuf::from("文档"),
        sort: SortSpec::default(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn desktop_context_and_background_icons_never_launch_commands() {
    let services = Arc::new(Services::default());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(services.clone(), sender)
            .with_desktop(services.clone())
            .run(receiver),
    );
    commands.send(Command::ListDesktopContext).await.unwrap();
    commands.send(navigation()).await.unwrap();
    let (mut context, mut listed, mut folder_icon) = (false, false, false);
    while !(context && listed && folder_icon) {
        match next(&mut events).await {
            Event::DesktopContext { shell, locations } => {
                assert_eq!(shell, ShellFlavor::PowerShell);
                assert_eq!(locations[0].path, Path::new("文档/🏠"));
                context = true;
            }
            Event::ListingFinished { entries, .. } => {
                assert_eq!(entries.len(), 150);
                listed = true;
            }
            Event::IconLoaded { key, pixels } if key == IconKey::folder() => {
                assert_eq!(
                    pixels.rgba.len(),
                    (pixels.width * pixels.height * 4) as usize
                );
                folder_icon = true;
            }
            _ => {}
        }
    }
    assert!(services.launches.lock().unwrap().is_empty());
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}

#[tokio::test]
async fn desktop_context_publishes_choices_before_loading_each_drive_and_special_icon() {
    let services = Arc::new(Services::default());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(services.clone(), sender)
            .with_desktop(services.clone())
            .run(receiver),
    );
    commands.send(Command::ListDesktopContext).await.unwrap();
    let mut context_published = false;
    let mut roots = std::collections::HashSet::new();
    let mut locations = std::collections::HashSet::new();
    while roots.len() < 2 || locations.len() < 4 {
        match next(&mut events).await {
            Event::DesktopContext { .. } => context_published = true,
            Event::IconLoaded { key, pixels } if key.drive_root.is_some() => {
                assert!(context_published, "drive icons arrived before locations");
                assert!(
                    pixels
                        .rgba
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|pixel| pixel[3] != 0)
                );
                roots.insert(key.drive_root.unwrap());
            }
            Event::IconLoaded { key, .. } => {
                if let IconKind::Location(kind) = key.kind {
                    assert!(context_published, "special icons arrived before locations");
                    locations.insert(kind);
                }
            }
            _ => {}
        }
    }
    assert!(roots.contains(Path::new("C:\\")));
    assert!(roots.contains(Path::new("D:\\")));
    assert!(services.launches.lock().unwrap().is_empty());
    assert!(services.opened.lock().unwrap().is_empty());
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn modal_properties_do_not_block_navigation_or_explicit_terminal_errors() {
    let services = Arc::new(Services::default());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let task = tokio::spawn(
        Engine::new(services.clone(), sender)
            .with_desktop(services.clone())
            .run(receiver),
    );
    commands
        .send(Command::ShowProperties {
            request: DesktopRequestId(1),
            paths: vec![PathBuf::from("文档/📁")],
        })
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(4),
        services.properties_entered.notified(),
    )
    .await
    .unwrap();
    commands.send(navigation()).await.unwrap();
    let terminal = TerminalRequest {
        command: "echo 'Žluťoučký 👩🏽‍💻'; $x".into(),
        working_directory: PathBuf::from("文档"),
        shell: ShellFlavor::PowerShell,
        mode: TerminalMode::Standard,
    };
    commands
        .send(Command::LaunchTerminal {
            request: DesktopRequestId(2),
            terminal: terminal.clone(),
        })
        .await
        .unwrap();
    let (mut listed, mut launched) = (false, false);
    while !(listed && launched) {
        match next(&mut events).await {
            Event::ListingFinished { .. } => listed = true,
            Event::DesktopActionFinished {
                request: DesktopRequestId(2),
                result: Err(_),
            } => launched = true,
            _ => {}
        }
    }
    {
        let recorded = services.launches.lock().unwrap();
        assert_eq!(recorded[0].command, terminal.command);
        assert_eq!(recorded[0].working_directory, terminal.working_directory);
    }
    *services.release_properties.0.lock().unwrap() = true;
    services.release_properties.1.notify_all();
    loop {
        if let Event::DesktopActionFinished {
            request: DesktopRequestId(1),
            result: Ok(DesktopActionResult::PropertiesInfo { body, .. }),
        } = next(&mut events).await
        {
            assert!(body.contains("17"));
            break;
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_default_open_does_not_block_navigation_and_keeps_the_exact_path() {
    let services = Arc::new(Services::default());
    let _release = ReleaseOpen(services.clone());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let task = tokio::spawn(
        Engine::new(services.clone(), sender)
            .with_desktop(services.clone())
            .run(receiver),
    );
    commands
        .send(Command::OpenEntry {
            request: DesktopRequestId(7),
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: "blocked-open".into(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), services.open_entered.notified())
        .await
        .unwrap();
    commands.send(navigation()).await.unwrap();
    loop {
        if matches!(
            next(&mut events).await,
            Event::ListingFinished {
                pane: PaneId::Right,
                ..
            }
        ) {
            break;
        }
    }
    let path = PathBuf::from("文档/Žluťoučký & echo $(never-execute);👩🏽‍💻.txt");
    commands
        .send(Command::OpenEntry {
            request: DesktopRequestId(8),
            pane: PaneId::Right,
            navigation: NavigationId(1),
            path: path.clone(),
        })
        .await
        .unwrap();
    loop {
        if matches!(
            next(&mut events).await,
            Event::DesktopActionFinished {
                request: DesktopRequestId(8),
                result: Ok(DesktopActionResult::FileOpened)
            }
        ) {
            break;
        }
    }
    assert!(services.opened.lock().unwrap().contains(&path));
    assert!(
        services.launches.lock().unwrap().is_empty(),
        "Opening a path must not use the command shell"
    );
    *services.release_open.0.lock().unwrap() = true;
    services.release_open.1.notify_all();
    loop {
        if matches!(
            next(&mut events).await,
            Event::DesktopActionFinished {
                request: DesktopRequestId(7),
                result: Ok(DesktopActionResult::FileOpened)
            }
        ) {
            break;
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}

#[tokio::test]
async fn linked_directory_is_internal_and_missing_file_is_a_contextual_error() {
    let services = Arc::new(Services::default());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(services.clone(), sender)
            .with_desktop(services)
            .run(receiver),
    );
    for (id, path) in [(3, "linked-folder"), (4, "missing-open")] {
        commands
            .send(Command::OpenEntry {
                request: DesktopRequestId(id),
                pane: PaneId::Right,
                navigation: NavigationId(42),
                path: path.into(),
            })
            .await
            .unwrap();
    }
    let mut done = 0;
    while done < 2 {
        match next(&mut events).await {
            Event::DesktopActionFinished {
                request: DesktopRequestId(3),
                result:
                    Ok(DesktopActionResult::OpenDirectory {
                        pane,
                        navigation,
                        path,
                    }),
            } => {
                assert_eq!(
                    (pane, navigation, path),
                    (PaneId::Right, NavigationId(42), "linked-folder".into())
                );
                done += 1;
            }
            Event::DesktopActionFinished {
                request: DesktopRequestId(4),
                result: Err(error),
            } => {
                assert_eq!(error.kind, io::ErrorKind::NotFound);
                assert_eq!(error.path, Path::new("missing-open"));
                done += 1;
            }
            _ => {}
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}

#[tokio::test]
async fn desktop_helpers_are_reaped_and_nonzero_status_is_reported() {
    let services = Arc::new(Services::default());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(services.clone(), sender)
            .with_desktop(services)
            .run(receiver),
    );
    for (id, path) in [(1, "helper-success"), (2, "helper-error")] {
        commands
            .send(Command::OpenEntry {
                request: DesktopRequestId(id),
                pane: PaneId::Left,
                navigation: NavigationId(1),
                path: path.into(),
            })
            .await
            .unwrap();
    }
    let mut completed = 0;
    while completed < 2 {
        match next(&mut events).await {
            Event::DesktopActionFinished {
                request: DesktopRequestId(1),
                result: Ok(DesktopActionResult::FileOpened),
            } => completed += 1,
            Event::DesktopActionFinished {
                request: DesktopRequestId(2),
                result: Err(error),
            } => {
                assert_eq!(error.path, Path::new("helper-error"));
                assert!(error.message.contains("Desktop opener exited"));
                completed += 1;
            }
            _ => {}
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_open_capacity_is_bounded_and_rejects_excess_without_dispatch() {
    let services = Arc::new(Services::default());
    let _release = ReleaseOpen(services.clone());
    let (commands, receiver) = mpsc::channel(16);
    let (sender, mut events) = mpsc::channel(32);
    let task = tokio::spawn(
        Engine::new(services.clone(), sender)
            .with_desktop(services.clone())
            .run(receiver),
    );
    for id in 1..=4 {
        commands
            .send(Command::OpenEntry {
                request: DesktopRequestId(id),
                pane: PaneId::Left,
                navigation: NavigationId(1),
                path: "blocked-open".into(),
            })
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(4), async {
        while services.opened.lock().unwrap().len() < 4 {
            services.open_entered.notified().await;
        }
    })
    .await
    .unwrap();
    commands
        .send(Command::OpenEntry {
            request: DesktopRequestId(5),
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: "must-not-dispatch".into(),
        })
        .await
        .unwrap();
    loop {
        if let Event::DesktopActionFinished {
            request: DesktopRequestId(5),
            result: Err(error),
        } = next(&mut events).await
        {
            assert!(error.message.contains("busy"));
            break;
        }
    }
    assert_eq!(services.opened.lock().unwrap().len(), 4);
    *services.release_open.0.lock().unwrap() = true;
    services.release_open.1.notify_all();
    let mut finished = 0;
    while finished < 4 {
        if matches!(
            next(&mut events).await,
            Event::DesktopActionFinished {
                result: Ok(DesktopActionResult::FileOpened),
                ..
            }
        ) {
            finished += 1;
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}
