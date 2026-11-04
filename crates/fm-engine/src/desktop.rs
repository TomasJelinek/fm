//! Bounded optional desktop work, independent of navigation and folder scans.

use crate::Outbox;
use fm_core::*;
use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    task::JoinSet,
};

const ICON_CAPACITY: usize = 256;
const MAX_TERMINALS: usize = 8;
const MAX_OPEN_HELPERS: usize = 16;

pub(crate) struct Desktop {
    services: Option<Arc<dyn DesktopServices>>,
    icons: mpsc::Sender<IconKey>,
    children: mpsc::Sender<TrackedChild>,
    property_budget: Arc<Semaphore>,
    launch_budget: Arc<Semaphore>,
    context_budget: Arc<Semaphore>,
    terminal_slots: Arc<Semaphore>,
    open_budget: Arc<Semaphore>,
    open_slots: Arc<Semaphore>,
    outbox: Outbox,
}

struct TrackedChild {
    child: Box<dyn TerminalProcess>,
    _slot: OwnedSemaphorePermit,
    completion: Option<(DesktopRequestId, PathBuf)>,
}

impl Desktop {
    pub(crate) fn new(
        services: Option<Arc<dyn DesktopServices>>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let (icons, receiver) = mpsc::channel(64);
        tasks.spawn(icon_worker(services.clone(), outbox.clone(), receiver));
        let (children, child_receiver) = mpsc::channel(MAX_TERMINALS);
        tasks.spawn(reap_children(child_receiver, outbox.clone()));
        let result = Self {
            services,
            icons,
            children,
            property_budget: Arc::new(Semaphore::new(1)),
            launch_budget: Arc::new(Semaphore::new(1)),
            context_budget: Arc::new(Semaphore::new(1)),
            terminal_slots: Arc::new(Semaphore::new(MAX_TERMINALS)),
            open_budget: Arc::new(Semaphore::new(4)),
            open_slots: Arc::new(Semaphore::new(MAX_OPEN_HELPERS)),
            outbox,
        };
        for key in [
            IconKey::file(),
            IconKey::folder(),
            IconKey::drive(),
            IconKey {
                kind: IconKind::Link,
                ..IconKey::file()
            },
        ] {
            let _ = result.icons.try_send(key);
        }
        result
    }

    pub(crate) fn icon_sender(&self) -> mpsc::Sender<IconKey> {
        self.icons.clone()
    }

    pub(crate) fn context(&self, tasks: &mut JoinSet<()>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        let Ok(permit) = self.context_budget.clone().try_acquire_owned() else {
            return;
        };
        let outbox = self.outbox.clone();
        let icons = self.icons.clone();
        tasks.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                services
                    .locations()
                    .map(|locations| (services.shell_flavor(), locations))
            })
            .await;
            let (event, drive_icons) = match result {
                Ok(Ok((shell, locations))) => {
                    let drive_icons: Vec<_> = locations
                        .iter()
                        .filter(|location| location.kind == LocationKind::Volume)
                        .map(|location| IconKey::for_drive(&location.path))
                        .collect();
                    (Event::DesktopContext { shell, locations }, drive_icons)
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
            if outbox.send(event).await {
                // Publish navigation roots first. Icon backpressure stays entirely
                // on the backend and shares the existing bounded/cache worker.
                for key in drive_icons {
                    if icons.send(key).await.is_err() {
                        break;
                    }
                }
            }
        });
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
        if terminal.command.trim().is_empty() || terminal.command.contains('\0') {
            self.fail(
                request,
                "Enter a nonempty command without NUL characters.",
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
                    Ok(DesktopActionResult::TerminalStarted { process_id })
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
                services.open_entry(&target)
            })
            .await;
            let result = match result {
                Ok(Ok(OpenEntryResult::Directory)) => Ok(DesktopActionResult::OpenDirectory {
                    pane,
                    navigation,
                    path,
                }),
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
        let outbox = self.outbox.clone();
        tasks.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                if let Some(services) = services
                    && services.show_properties(&paths)?
                {
                    return Ok(DesktopActionResult::PropertiesOpened);
                }
                metadata_properties(filesystem.as_ref(), &paths)
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

fn metadata_properties(
    filesystem: &dyn FileSystem,
    paths: &[PathBuf],
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
    let body = format!(
        "{} items\n{bytes} bytes in {files} regular files\n{unknown} items without a file length\n\nRead-only metadata. Folder contents are not included.",
        paths.len()
    );
    Ok(DesktopActionResult::PropertiesInfo { title, body })
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
                })
                .unwrap_or_else(|| fallback_icon(worker_key.kind))
        })
        .await;
        let pixels = result.unwrap_or_else(|_| fallback_icon(kind));
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

fn fallback_icon(kind: IconKind) -> IconPixels {
    // Small backend-generated RGBA assets also work without a font icon glyph.
    let mut rgba = vec![0; 24 * 24 * 4];
    let color = match kind {
        IconKind::Folder => [232, 182, 62, 255],
        IconKind::Drive => [126, 144, 163, 255],
        IconKind::Link => [94, 173, 155, 255],
        IconKind::File => [181, 200, 220, 255],
    };
    for y in 4..21 {
        for x in 3..21 {
            let i = (y * 24 + x) * 4;
            rgba[i..i + 4].copy_from_slice(&color);
        }
    }
    IconPixels {
        width: 24,
        height: 24,
        rgba: rgba.into(),
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
