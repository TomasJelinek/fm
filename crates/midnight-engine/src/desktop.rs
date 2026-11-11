//! Bounded optional desktop work, independent of navigation and folder scans.

use crate::Outbox;
use midnight_core::*;
use std::{
    collections::{HashSet, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch},
    task::JoinSet,
};

#[path = "desktop_editor.rs"]
mod editor;

const ICON_CAPACITY: usize = 256;
const MAX_TERMINALS: usize = 8;
const MAX_OPEN_HELPERS: usize = 16;

pub(crate) struct Desktop {
    locale: Option<Arc<dyn LocaleService>>,
    services: Option<Arc<dyn DesktopServices>>,
    icons: mpsc::Sender<IconKey>,
    volume_icons: watch::Sender<Vec<IconKey>>,
    children: mpsc::Sender<TrackedChild>,
    property_budget: Arc<Semaphore>,
    launch_budget: Arc<Semaphore>,
    context_requests: watch::Sender<u64>,
    terminal_slots: Arc<Semaphore>,
    open_budget: Arc<Semaphore>,
    open_slots: Arc<Semaphore>,
    outbox: Outbox,
}

pub(crate) struct TrackedChild {
    pub(crate) child: Box<dyn TerminalProcess>,
    pub(crate) _slot: OwnedSemaphorePermit,
    pub(crate) completion: Option<(DesktopRequestId, PathBuf)>,
}

impl Desktop {
    pub(crate) fn new(
        services: Option<Arc<dyn DesktopServices>>,
        locale: Option<Arc<dyn LocaleService>>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let (icons, receiver) = mpsc::channel(64);
        tasks.spawn(icon_worker(services.clone(), outbox.clone(), receiver));
        let (volume_icons, roots) = watch::channel(Vec::new());
        tasks.spawn(forward_icons(roots, icons.clone()));
        let (location_icons, locations) = watch::channel(Vec::new());
        tasks.spawn(forward_icons(locations, icons.clone()));
        let (context_requests, context_receiver) = watch::channel(0);
        tasks.spawn(context_worker(
            services.clone(),
            locale.clone(),
            outbox.clone(),
            location_icons,
            context_receiver,
        ));
        let (children, child_receiver) = mpsc::channel(MAX_TERMINALS);
        tasks.spawn(reap_children(child_receiver, outbox.clone()));
        let result = Self {
            locale,
            services,
            icons,
            volume_icons,
            children,
            property_budget: Arc::new(Semaphore::new(1)),
            launch_budget: Arc::new(Semaphore::new(1)),
            context_requests,
            terminal_slots: Arc::new(Semaphore::new(MAX_TERMINALS)),
            open_budget: Arc::new(Semaphore::new(4)),
            open_slots: Arc::new(Semaphore::new(MAX_OPEN_HELPERS)),
            outbox,
        };
        for key in [
            IconKey::file(),
            IconKey::folder(),
            IconKey::folder_link(),
            IconKey::drive(),
            IconKey {
                kind: IconKind::Link,
                ..IconKey::file()
            },
        ]
        .into_iter()
        .chain(ActionIcon::ALL.into_iter().map(IconKey::action))
        {
            let _ = result.icons.try_send(key);
        }
        result
    }

    pub(crate) fn icon_sender(&self) -> mpsc::Sender<IconKey> {
        self.icons.clone()
    }

    pub(crate) fn command_runtime(&self) -> crate::user_command_runtime::Runtime {
        crate::user_command_runtime::Runtime::new(
            self.launch_budget.clone(),
            self.terminal_slots.clone(),
            self.children.clone(),
        )
    }

    pub(crate) fn volume_icon_sender(&self) -> watch::Sender<Vec<IconKey>> {
        self.volume_icons.clone()
    }

    pub(crate) fn context_sender(&self) -> watch::Sender<u64> {
        self.context_requests.clone()
    }

    pub(crate) fn context(&self) {
        self.context_requests
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    pub(crate) fn launch(
        &self,
        request: DesktopRequestId,
        terminal: TerminalRequest,
        tasks: &mut JoinSet<()>,
    ) {
        let Some(services) = self.services.clone() else {
            self.fail(request, "Terminal integration is unavailable.", tasks);
            return;
        };
        // Exactly empty opens an interactive shell. Whitespace is still an invalid
        // submitted command, and neither launch path accepts an embedded NUL.
        if (!terminal.command.is_empty() && terminal.command.trim().is_empty())
            || terminal.command.contains('\0')
        {
            self.fail(
                request,
                "Terminal commands cannot contain NUL or consist only of whitespace.",
                tasks,
            );
            return;
        }
        let Ok(permit) = self.launch_budget.clone().try_acquire_owned() else {
            self.fail(request, "A terminal is already being started.", tasks);
            return;
        };
        let Ok(slot) = self.terminal_slots.clone().try_acquire_owned() else {
            self.fail(
                request,
                "Eight terminal processes are already active.",
                tasks,
            );
            return;
        };
        let outbox = self.outbox.clone();
        let children = self.children.clone();
        tasks.spawn(async move {
            let path = terminal.working_directory.clone();
            let history = CommandHistoryEntry {
                command: terminal.command.clone(),
                shell: terminal.shell,
            };
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                services.launch_terminal(&terminal)
            })
            .await;
            let result = match result {
                Ok(Ok(child)) => {
                    let process_id = child.id();
                    // The bounded reaper retains the slot until the process exits.
                    let _ = children
                        .send(TrackedChild {
                            child,
                            _slot: slot,
                            completion: None,
                        })
                        .await;
                    Ok(DesktopActionResult::TerminalStarted {
                        process_id,
                        history,
                    })
                }
                Ok(Err(error)) => Err(error),
                Err(error) => Err(worker_error("launch terminal", &path, error)),
            };
            outbox
                .send(Event::DesktopActionFinished { request, result })
                .await;
        });
    }

    pub(crate) fn open_entry(
        &self,
        request: DesktopRequestId,
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        directory_only: bool,
        tasks: &mut JoinSet<()>,
    ) {
        let Some(services) = self.services.clone() else {
            self.fail(request, "Desktop file opening is unavailable.", tasks);
            return;
        };
        let Ok(permit) = self.open_budget.clone().try_acquire_owned() else {
            self.fail(request, "File opening is busy. Try again.", tasks);
            return;
        };
        let Ok(slot) = self.open_slots.clone().try_acquire_owned() else {
            self.fail(
                request,
                "Too many desktop file-opening helpers are active.",
                tasks,
            );
            return;
        };
        let outbox = self.outbox.clone();
        let children = self.children.clone();
        tasks.spawn(async move {
            let target = path.clone();
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                if directory_only {
                    services
                        .directory_destination(&target)
                        .map(|path| OpenEntryResult::DirectoryTarget { path })
                } else {
                    services.open_entry(&target)
                }
            })
            .await;
            let result = match result {
                Ok(Ok(OpenEntryResult::Directory)) => Ok(DesktopActionResult::OpenDirectory {
                    pane,
                    navigation,
                    path,
                }),
                Ok(Ok(OpenEntryResult::DirectoryTarget { path: target })) => {
                    Ok(DesktopActionResult::OpenDirectoryTarget {
                        pane,
                        navigation,
                        path,
                        target,
                    })
                }
                Ok(Ok(OpenEntryResult::File { process: None })) => {
                    Ok(DesktopActionResult::FileOpened)
                }
                Ok(Ok(OpenEntryResult::File {
                    process: Some(child),
                })) => {
                    // Helpers such as xdg-open may report association errors on
                    // exit. Reap them off the UI thread and report that result.
                    let _ = children
                        .send(TrackedChild {
                            child,
                            _slot: slot,
                            completion: Some((request, path)),
                        })
                        .await;
                    return;
                }
                Ok(Err(error)) => Err(error),
                Err(error) => Err(worker_error("open entry", &path, error)),
            };
            outbox
                .send(Event::DesktopActionFinished { request, result })
                .await;
        });
    }

    pub(crate) fn properties(
        &self,
        filesystem: Arc<dyn FileSystem>,
        request: DesktopRequestId,
        paths: Vec<PathBuf>,
        tasks: &mut JoinSet<()>,
    ) {
        if paths.is_empty() {
            return;
        }
        let Ok(permit) = self.property_budget.clone().try_acquire_owned() else {
            self.fail(request, "A properties request is already open.", tasks);
            return;
        };
        let services = self.services.clone();
        let locale = self.locale.clone();
        let outbox = self.outbox.clone();
        tasks.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                if let Some(services) = services
                    && services.show_properties(&paths)?
                {
                    return Ok(DesktopActionResult::PropertiesOpened);
                }
                let locale = locale.map_or_else(
                    || Ok(NumberLocale::default()),
                    |service| service.number_locale(),
                )?;
                metadata_properties(filesystem.as_ref(), &paths, &locale)
            })
            .await;
            let result = match result {
                Ok(result) => result,
                Err(error) => Err(worker_error("properties", Path::new(""), error)),
            };
            outbox
                .send(Event::DesktopActionFinished { request, result })
                .await;
        });
    }

    pub(crate) async fn resolve_editor(
        &self,
        services: Option<Arc<dyn EditorService>>,
        tasks: &mut JoinSet<()>,
    ) {
        let outbox = self.outbox.clone();
        let Ok(permit) = self.launch_budget.clone().try_acquire_owned() else {
            outbox
                .send(Event::EditorDefaultResolved(Err(editor_error(
                    "Editor discovery is busy. Reopen Options to retry.",
                ))))
                .await;
            return;
        };
        let Some(services) = services else {
            outbox
                .send(Event::EditorDefaultResolved(Err(editor_error(
                    "External editing is unavailable.",
                ))))
                .await;
            return;
        };
        tasks.spawn(async move {
            let result =
                tokio::task::spawn_blocking(move || (services.default_editor(), permit)).await;
            let (result, _permit) = match result {
                Ok((result, permit)) => (result, Some(permit)),
                Err(cause) => (Err(worker_error("find editor", Path::new(""), cause)), None),
            };
            outbox.send(Event::EditorDefaultResolved(result)).await;
        });
    }

    async fn editor_failure(&self, request: DesktopRequestId, message: &'static str) {
        self.outbox
            .send(Event::DesktopActionFinished {
                request,
                result: Err(editor_error(message)),
            })
            .await;
    }

    fn fail(&self, request: DesktopRequestId, message: &'static str, tasks: &mut JoinSet<()>) {
        let outbox = self.outbox.clone();
        tasks.spawn(async move {
            outbox
                .send(Event::DesktopActionFinished {
                    request,
                    result: Err(FsError::from_io(
                        "desktop action",
                        Path::new(""),
                        io::Error::other(message),
                    )),
                })
                .await;
        });
    }
}

fn editor_error(message: &'static str) -> FsError {
    FsError::from_io("external editor", Path::new(""), io::Error::other(message))
}

fn metadata_properties(
    filesystem: &dyn FileSystem,
    paths: &[PathBuf],
    locale: &NumberLocale,
) -> Result<DesktopActionResult, FsError> {
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut unknown = 0usize;
    for path in paths {
        match filesystem.entry_metadata(path) {
            Ok(metadata) => match metadata.size {
                Some(size) => {
                    bytes = bytes.checked_add(size).ok_or_else(|| {
                        FsError::from_io(
                            "properties",
                            path,
                            io::Error::other("Combined size exceeds u64."),
                        )
                    })?;
                    files += 1;
                }
                None => unknown += 1,
            },
            Err(error) if paths.len() == 1 => return Err(error),
            Err(_) => unknown += 1,
        }
    }
    let title = if paths.len() == 1 {
        paths[0].display().to_string()
    } else {
        format!("{} selected items", paths.len())
    };
    let bytes = locale.format_u64(bytes);
    let body = format!(
        "{} items\n{bytes} bytes in {files} regular files\n{unknown} items without a file length\n\nRead-only metadata. Folder contents are not included.",
        paths.len()
    );
    Ok(DesktopActionResult::PropertiesInfo { title, body })
}

/// A watch slot coalesces refreshes without overlapping uninterruptible host calls.
async fn context_worker(
    services: Option<Arc<dyn DesktopServices>>,
    locale: Option<Arc<dyn LocaleService>>,
    outbox: Outbox,
    icons: watch::Sender<Vec<IconKey>>,
    mut requests: watch::Receiver<u64>,
) {
    let Some(services) = services else {
        return;
    };
    while requests.changed().await.is_ok() {
        let version = *requests.borrow_and_update();
        let worker = services.clone();
        let locale = locale.clone();
        let result = tokio::task::spawn_blocking(move || {
            worker
                .locations()
                .and_then(|locations| {
                    crate::navigation_ordering::locations(locations, locale.as_deref())
                })
                .map(|locations| (worker.shell_flavor(), locations))
        })
        .await;
        let (event, keys) = match result {
            Ok(Ok((shell, (locations, network_position)))) => {
                // Keep the finite semantic aliases ahead of large mount inventories.
                let keys = locations
                    .iter()
                    .filter(|location| location.kind != LocationKind::Volume)
                    .chain(
                        locations
                            .iter()
                            .filter(|location| location.kind == LocationKind::Volume),
                    )
                    .take(ICON_CAPACITY)
                    .map(IconKey::for_location)
                    .collect();
                (
                    Event::DesktopContext {
                        shell,
                        locations,
                        network_position,
                    },
                    keys,
                )
            }
            Ok(Err(error)) => (Event::DesktopContextFailed(error), Vec::new()),
            Err(error) => (
                Event::DesktopContextFailed(worker_error(
                    "desktop locations",
                    Path::new(""),
                    error,
                )),
                Vec::new(),
            ),
        };
        // Recheck after backpressure. No obsolete result, success or failure,
        // may replace the typed metadata requested for the latest inventory.
        let Ok(permit) = outbox.sender.reserve().await else {
            return;
        };
        {
            let latest = requests.borrow();
            if version != *latest {
                continue;
            }
            permit.send(event);
            icons.send_replace(keys);
        }
        (outbox.wakeup)();
    }
}

async fn icon_worker(
    services: Option<Arc<dyn DesktopServices>>,
    outbox: Outbox,
    mut receiver: mpsc::Receiver<IconKey>,
) {
    let mut cached = HashSet::new();
    while let Some(key) = receiver.recv().await {
        if cached.contains(&key) || cached.len() == ICON_CAPACITY {
            continue;
        }
        cached.insert(key.clone());
        let service = services.clone();
        let worker_key = key.clone();
        let kind = key.kind;
        let result = tokio::task::spawn_blocking(move || {
            service
                .and_then(|service| service.load_icon(&worker_key).ok().flatten())
                .filter(|pixels| {
                    (1..=64).contains(&pixels.width)
                        && (1..=64).contains(&pixels.height)
                        && pixels.rgba.len() == pixels.width as usize * pixels.height as usize * 4
                        && pixels
                            .rgba
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .any(|pixel| pixel[3] != 0)
                })
                .or_else(|| fallback_icon(worker_key.kind))
        })
        .await;
        let Some(pixels) = result.unwrap_or_else(|_| fallback_icon(kind)) else {
            // Missing semantic roles keep the UI's appropriate vector fallback.
            // The key remains cached so an unsupported role is never queried again.
            continue;
        };
        if !outbox
            .send(Event::IconLoaded {
                key,
                pixels: Arc::new(pixels),
            })
            .await
        {
            break;
        }
    }
}

fn fallback_icon(kind: IconKind) -> Option<IconPixels> {
    if matches!(kind, IconKind::Link | IconKind::FolderLink) {
        return Some(link_fallback(kind == IconKind::FolderLink));
    }
    // Small backend-generated RGBA assets also work without a font icon glyph.
    let color = match kind {
        IconKind::Folder => [232, 182, 62, 255],
        IconKind::Drive => [126, 144, 163, 255],
        IconKind::Link | IconKind::FolderLink => unreachable!("link icons use a badge"),
        IconKind::File => [181, 200, 220, 255],
        IconKind::Action(_) | IconKind::Location(_) => return None,
    };
    let mut rgba = vec![0; 24 * 24 * 4];
    for y in 4..21 {
        for x in 3..21 {
            let i = (y * 24 + x) * 4;
            rgba[i..i + 4].copy_from_slice(&color);
        }
    }
    Some(IconPixels {
        width: 24,
        height: 24,
        rgba: rgba.into(),
        generic: kind == IconKind::File,
    })
}

fn link_fallback(folder: bool) -> IconPixels {
    const EDGE: usize = 24;
    let mut rgba = vec![0; EDGE * EDGE * 4];
    let mut paint = |left: usize, top: usize, right: usize, bottom: usize, color: [u8; 4]| {
        for y in top..bottom {
            for x in left..right {
                let offset = (y * EDGE + x) * 4;
                rgba[offset..offset + 4].copy_from_slice(&color);
            }
        }
    };
    if folder {
        paint(3, 4, 12, 8, [197, 140, 36, 255]);
        paint(2, 7, 23, 21, [197, 140, 36, 255]);
        paint(3, 8, 22, 20, [244, 193, 72, 255]);
    } else {
        paint(6, 2, 21, 22, [91, 112, 137, 255]);
        paint(7, 3, 20, 21, [224, 234, 245, 255]);
        paint(16, 3, 20, 7, [165, 188, 214, 255]);
    }
    // A bordered arrow badge remains recognizable without host art or icon fonts.
    paint(0, 13, 13, 24, [43, 73, 107, 255]);
    paint(1, 14, 12, 23, [250, 252, 255, 255]);
    paint(5, 15, 10, 17, [26, 93, 174, 255]);
    paint(8, 15, 10, 20, [26, 93, 174, 255]);
    paint(6, 17, 8, 19, [26, 93, 174, 255]);
    paint(4, 19, 6, 21, [26, 93, 174, 255]);
    paint(2, 20, 4, 22, [26, 93, 174, 255]);
    IconPixels {
        width: EDGE as u32,
        height: EDGE as u32,
        rgba: rgba.into(),
        generic: false,
    }
}

/// Coalesce discovery while native lookup is busy, without delaying volume metadata.
async fn forward_icons(mut roots: watch::Receiver<Vec<IconKey>>, icons: mpsc::Sender<IconKey>) {
    let mut pending = VecDeque::new();
    loop {
        tokio::select! {
            biased;
            changed = roots.changed() => {
                if changed.is_err() { return; }
                // Producer and consumer both bound this snapshot independently.
                pending = roots.borrow_and_update().iter().take(ICON_CAPACITY).cloned().collect();
            }
            permit = icons.reserve(), if !pending.is_empty() => {
                let Ok(permit) = permit else { return; };
                if let Some(key) = pending.pop_front() { permit.send(key); }
            }
            _ = icons.closed() => return,
        }
    }
}

async fn reap_children(mut receiver: mpsc::Receiver<TrackedChild>, outbox: Outbox) {
    let mut children = Vec::<TrackedChild>::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            child = receiver.recv() => match child {
                Some(child) => children.push(child),
                None => return,
            },
            _ = tick.tick(), if !children.is_empty() => {
                let result = tokio::task::spawn_blocking(move || poll_children(children)).await;
                let Ok((remaining, completed)) = result else { return; };
                children = remaining;
                for event in completed {
                    if !outbox.send(event).await { return; }
                }
            }
        }
    }
}

fn poll_children(mut children: Vec<TrackedChild>) -> (Vec<TrackedChild>, Vec<Event>) {
    let mut completed = Vec::new();
    children.retain_mut(|child| match child.child.try_wait() {
        Ok(Some(status)) => {
            if let Some((request, path)) = child.completion.take() {
                let result = if status.success() {
                    Ok(DesktopActionResult::FileOpened)
                } else {
                    Err(FsError::from_io(
                        "open entry",
                        &path,
                        io::Error::other(format!("Desktop opener exited with {status}")),
                    ))
                };
                completed.push(Event::DesktopActionFinished { request, result });
            }
            false
        }
        Ok(None) => true,
        Err(error) => {
            if let Some((request, path)) = child.completion.take() {
                completed.push(Event::DesktopActionFinished {
                    request,
                    result: Err(FsError::from_io("wait for desktop opener", &path, error)),
                });
            }
            true
        }
    });
    (children, completed)
}

fn worker_error(operation: &'static str, path: &Path, error: tokio::task::JoinError) -> FsError {
    FsError::from_io(operation, path, io::Error::other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::{Future, poll_fn};
    use std::sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Notify;

    #[test]
    fn link_fallbacks_have_a_recognizable_badge_and_distinct_file_or_folder_bases() {
        let folder = fallback_icon(IconKind::Folder).unwrap();
        let folder_link = fallback_icon(IconKind::FolderLink).unwrap();
        let link = fallback_icon(IconKind::Link).unwrap();
        assert_ne!(
            folder_link.rgba, folder.rgba,
            "a junction must not look like an ordinary folder"
        );
        assert_ne!(
            folder_link.rgba, link.rgba,
            "folder/file link bases must differ"
        );
        for icon in [folder_link, link] {
            let colors: HashSet<_> = icon.rgba.as_chunks::<4>().0.iter().copied().collect();
            assert!(
                colors.len() >= 4,
                "link fallback needs a visible base, border and arrow"
            );
            assert!(colors.contains(&[0, 0, 0, 0]));
        }
    }

    #[derive(Default)]
    struct Icons {
        keys: Mutex<Vec<IconKey>>,
        roots: Mutex<Vec<Volume>>,
        gate: Option<Arc<IconGate>>,
        context_gate: Option<Arc<IconGate>>,
        context_calls: AtomicUsize,
    }

    #[derive(Default)]
    struct IconGate {
        entered: Notify,
        released: Mutex<bool>,
        wake: Condvar,
    }

    impl IconGate {
        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.wake.notify_all();
        }
    }

    struct ReleaseIcon(Arc<IconGate>);

    impl Drop for ReleaseIcon {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    impl IconService for Icons {
        fn load_icon(&self, key: &IconKey) -> Result<Option<IconPixels>, FsError> {
            self.keys.lock().unwrap().push(key.clone());
            if key.kind == IconKind::Action(ActionIcon::Options)
                && let Some(gate) = &self.gate
            {
                gate.entered.notify_one();
                let mut released = gate.released.lock().unwrap();
                while !*released {
                    released = gate.wake.wait(released).unwrap();
                }
            }
            Ok(match key.kind {
                IconKind::Action(ActionIcon::Options) => {
                    Some(IconPixels::new(1, 1, vec![12, 34, 56, 128]).unwrap())
                }
                IconKind::Action(ActionIcon::Help) => {
                    Some(IconPixels::new(1, 1, vec![0; 4]).unwrap())
                }
                IconKind::FolderLink => Some(IconPixels::new(1, 1, vec![0; 4]).unwrap()),
                IconKind::Action(ActionIcon::View) => Some(IconPixels {
                    width: 65,
                    height: 1,
                    rgba: vec![255; 260].into(),
                    generic: false,
                }),
                _ => None,
            })
        }
    }
    impl TerminalService for Icons {
        fn shell_flavor(&self) -> ShellFlavor {
            ShellFlavor::Posix
        }
        fn launch_terminal(
            &self,
            _: &TerminalRequest,
        ) -> Result<Box<dyn TerminalProcess>, FsError> {
            unreachable!("icon lookup must not launch a terminal")
        }
    }
    impl PropertyService for Icons {
        fn show_properties(&self, _: &[PathBuf]) -> Result<bool, FsError> {
            unreachable!("icon lookup must not open properties")
        }
    }
    impl LocationService for Icons {
        fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
            let locations = self
                .roots
                .lock()
                .unwrap()
                .iter()
                .map(|volume| NavigationLocation {
                    id: volume.id.clone(),
                    label: volume.label.clone(),
                    path: volume.root.clone(),
                    kind: LocationKind::Volume,
                    // Deliberately unrelated to path syntax: this is native metadata.
                    drive_letter: Some('Z'),
                })
                .collect();
            let call = self.context_calls.fetch_add(1, Ordering::SeqCst);
            if call == 0
                && let Some(gate) = &self.context_gate
            {
                gate.entered.notify_one();
                let mut released = gate.released.lock().unwrap();
                while !*released {
                    released = gate.wake.wait(released).unwrap();
                }
            }
            Ok(locations)
        }
    }
    impl OpenService for Icons {
        fn open_entry(&self, _: &Path) -> Result<OpenEntryResult, FsError> {
            unreachable!("icon lookup must not open files")
        }
    }

    impl DirectoryService for Icons {
        fn read_directory(
            &self,
            _: &Path,
            _: &Cancellation,
            _: &mut dyn FnMut(Entry) -> bool,
        ) -> Result<(), FsError> {
            Ok(())
        }
        fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
            unreachable!("empty test directories have no entries")
        }
    }
    impl VolumeService for Icons {
        fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
            Ok(self.roots.lock().unwrap().clone())
        }
    }
    impl StorageService for Icons {
        fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
            Ok(SpaceInfo {
                available: 4,
                total: 8,
            })
        }
    }

    #[tokio::test]
    async fn unsupported_actions_and_locations_keep_negative_cached_semantic_fallbacks() {
        let services = Arc::new(Icons::default());
        let (sender, mut events) = mpsc::channel(16);
        let outbox = Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        };
        let (icons, receiver) = mpsc::channel(16);
        let worker = tokio::spawn(icon_worker(Some(services.clone()), outbox, receiver));
        let missing = IconKey::action(ActionIcon::Back);
        let native = IconKey::action(ActionIcon::Options);
        let home = IconKey {
            kind: IconKind::Location(LocationKind::Home),
            ..IconKey::folder()
        };
        for key in [
            missing.clone(),
            native.clone(),
            missing.clone(),
            IconKey::action(ActionIcon::Help),
            IconKey::action(ActionIcon::View),
            home.clone(),
            home.clone(),
            IconKey {
                kind: IconKind::Location(LocationKind::Downloads),
                ..IconKey::folder()
            },
            IconKey::folder(),
        ] {
            icons.send(key).await.unwrap();
        }
        drop(icons);
        worker.await.unwrap();
        let mut emitted = HashSet::new();
        while let Some(Event::IconLoaded { key, pixels }) = events.recv().await {
            if key == native {
                assert_eq!(&*pixels.rgba, [12, 34, 56, 128]);
            }
            assert_ne!(
                key, missing,
                "generic bitmap replaced the semantic Back fallback"
            );
            emitted.insert(key);
        }
        assert_eq!(emitted, HashSet::from([native, IconKey::folder()]));
        for negative in [missing, home] {
            assert_eq!(
                services
                    .keys
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|key| **key == negative)
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn transparent_host_link_art_uses_and_caches_the_recognizable_fallback() {
        let services = Arc::new(Icons::default());
        let (sender, mut events) = mpsc::channel(2);
        let outbox = Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        };
        let (icons, receiver) = mpsc::channel(2);
        let worker = tokio::spawn(icon_worker(Some(services.clone()), outbox, receiver));
        icons.send(IconKey::folder_link()).await.unwrap();
        icons.send(IconKey::folder_link()).await.unwrap();
        drop(icons);
        worker.await.unwrap();
        match events.recv().await.unwrap() {
            Event::IconLoaded { key, pixels } => {
                assert_eq!(key, IconKey::folder_link());
                assert_eq!(pixels.rgba, link_fallback(true).rgba);
            }
            event => panic!("unexpected icon event: {event:?}"),
        }
        assert!(events.recv().await.is_none());
        assert_eq!(*services.keys.lock().unwrap(), [IconKey::folder_link()]);
    }

    #[tokio::test]
    async fn hotplug_discovery_does_not_wait_for_icons_and_reused_ids_keep_raw_root_keys() {
        let gate = Arc::new(IconGate::default());
        let _release = ReleaseIcon(gate.clone());
        let services = Arc::new(Icons {
            gate: Some(gate.clone()),
            ..Icons::default()
        });
        let (commands, receiver) = mpsc::channel(8);
        let (sender, mut events) = mpsc::channel(64);
        let task = tokio::spawn(
            crate::Engine::new(services.clone(), sender)
                .with_desktop(services.clone())
                .run(receiver),
        );
        tokio::time::timeout(Duration::from_secs(4), gate.entered.notified())
            .await
            .unwrap();
        let first = PathBuf::from("/native/文档-old");
        let second = PathBuf::from("/native/👩🏽‍💻-new");
        for root in [&first, &second] {
            *services.roots.lock().unwrap() = vec![Volume {
                id: "reused-id".into(),
                label: "same label".into(),
                root: root.clone(),
            }];
            commands.send(Command::ListVolumes).await.unwrap();
            loop {
                let event = tokio::time::timeout(Duration::from_secs(4), events.recv())
                    .await
                    .unwrap()
                    .unwrap();
                if let Event::VolumesUpdated(volumes) = event {
                    assert_eq!(volumes.len(), 1);
                    assert_eq!(&volumes[0].root, root);
                    break;
                }
            }
        }
        gate.release();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(4), events.recv())
                .await
                .unwrap()
                .unwrap();
            if let Event::IconLoaded { key, .. } = event
                && key == IconKey::for_drive(&second)
            {
                break;
            }
        }
        {
            let keys = services.keys.lock().unwrap();
            assert!(keys.contains(&IconKey::for_drive(&second)));
        }
        assert_ne!(IconKey::for_drive(&first), IconKey::for_drive(&second));
        commands.send(Command::Shutdown).await.unwrap();
        drop(events);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn latest_volume_roots_replace_pending_icons_under_channel_backpressure() {
        let (roots, receiver) = watch::channel(Vec::new());
        let (icons, mut received) = mpsc::channel(1);
        let sentinel = IconKey::folder();
        icons.send(sentinel.clone()).await.unwrap();
        let forwarder = forward_icons(receiver, icons);
        tokio::pin!(forwarder);
        let latest = IconKey::for_drive(Path::new("/native/新しい-volume"));
        for key in [
            IconKey::for_drive(Path::new("/native/removed")),
            latest.clone(),
        ] {
            roots.send_replace(vec![key]);
            poll_fn(|context| {
                assert!(forwarder.as_mut().poll(context).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
        }
        assert_eq!(received.recv().await, Some(sentinel));
        let forwarded = tokio::select! {
            biased;
            _ = &mut forwarder => panic!("forwarder stopped while receivers were open"),
            key = received.recv() => key,
        };
        assert_eq!(forwarded, Some(latest));
        assert!(received.try_recv().is_err());
        received.close();
        tokio::time::timeout(Duration::from_secs(1), forwarder)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn hotplug_during_native_context_lookup_coalesces_to_current_typed_metadata() {
        let gate = Arc::new(IconGate::default());
        let _release = ReleaseIcon(gate.clone());
        let old = PathBuf::from("/native/old");
        let current = PathBuf::from("/native/新しい-drive");
        let volume = |root: PathBuf| Volume {
            id: "same-id".into(),
            label: "native name".into(),
            root,
        };
        let services = Arc::new(Icons {
            roots: Mutex::new(vec![volume(old)]),
            context_gate: Some(gate.clone()),
            ..Icons::default()
        });
        let (commands, receiver) = mpsc::channel(8);
        let (sender, mut events) = mpsc::channel(32);
        let task = tokio::spawn(
            crate::Engine::new(services.clone(), sender)
                .with_desktop(services.clone())
                .run(receiver),
        );
        commands.send(Command::ListDesktopContext).await.unwrap();
        tokio::time::timeout(Duration::from_secs(4), gate.entered.notified())
            .await
            .unwrap();
        *services.roots.lock().unwrap() = vec![volume(current.clone())];
        commands.send(Command::ListVolumes).await.unwrap();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(4), events.recv())
                .await
                .unwrap()
                .unwrap();
            if let Event::VolumesUpdated(volumes) = event {
                assert_eq!(volumes[0].root, current);
                break;
            }
        }
        assert_eq!(services.context_calls.load(Ordering::SeqCst), 1);
        gate.release();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(4), events.recv())
                .await
                .unwrap()
                .unwrap();
            if let Event::DesktopContext { locations, .. } = event {
                assert_eq!(locations.len(), 1);
                assert_eq!(locations[0].path, current, "stale context was published");
                assert_eq!(locations[0].drive_letter, Some('Z'));
                break;
            }
        }
        assert_eq!(services.context_calls.load(Ordering::SeqCst), 2);
        commands.send(Command::Shutdown).await.unwrap();
        drop(events);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn icon_cache_deduplicates_and_stops_at_256_types() {
        let (sender, mut events) = mpsc::channel(512);
        let outbox = Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        };
        let (icons, receiver) = mpsc::channel(512);
        let worker = tokio::spawn(icon_worker(None, outbox, receiver));
        for i in 0..300 {
            let key = IconKey {
                extension: Some(format!("type-{i}").into()),
                ..IconKey::file()
            };
            icons.send(key.clone()).await.unwrap();
            icons.send(key).await.unwrap();
        }
        drop(icons);
        worker.await.unwrap();
        let mut keys = HashSet::new();
        while let Some(Event::IconLoaded { key, pixels }) = events.recv().await {
            assert!(keys.insert(key));
            assert_eq!(
                pixels.rgba.len(),
                (pixels.width * pixels.height * 4) as usize
            );
        }
        assert_eq!(keys.len(), 256);
    }
}
