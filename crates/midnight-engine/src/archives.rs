//! Archive navigation occupies bounded backend workers and carries only provider IDs.
use crate::{Outbox, Snapshots};
use midnight_core::*;
use midnight_filesystem_host::{Catalog, Mount};
use std::{
    path::Path,
    sync::{Arc, OnceLock},
};
use tokio::{
    sync::{Notify, Semaphore},
    task::JoinSet,
};
#[cfg(test)]
mod tests;

/// Backend capture authority. An expected revision must be validated before returning content.
pub type ArchiveCapture = Arc<
    dyn Fn(
            &Path,
            Option<&[u8]>,
            &Cancellation,
        ) -> Result<midnight_plugin_host::CapturedFile, FsError>
        + Send
        + Sync,
>;

fn capture_source(
    capture: Option<&ArchiveCapture>,
    path: &Path,
    expected: Option<&[u8]>,
    cancel: &Cancellation,
) -> Result<midnight_plugin_host::CapturedFile, FsError> {
    let interrupted = || {
        FsError::from_io(
            "capture archive",
            path,
            std::io::ErrorKind::Interrupted.into(),
        )
    };
    if cancel.is_cancelled() {
        return Err(interrupted());
    }
    let capture = capture.ok_or_else(|| {
        FsError::from_io(
            "capture archive",
            path,
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Native filesystem content capture is unavailable",
            ),
        )
    })?;
    let source = capture(path, expected, cancel)?;
    if cancel.is_cancelled() {
        return Err(interrupted());
    }
    Ok(source)
}

pub(crate) struct Controller {
    capture: Option<ArchiveCapture>,
    catalog: Arc<OnceLock<Catalog>>,
    budget: Arc<Semaphore>,
    cancellations: [Cancellation; 2],
    outbox: Outbox,
    snapshots: Snapshots,
    ready: Arc<Notify>,
}
impl Controller {
    pub(crate) fn new(
        capture: Option<ArchiveCapture>,
        outbox: Outbox,
        snapshots: Snapshots,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let catalog = Arc::new(OnceLock::new());
        let ready = Arc::new(Notify::new());
        let result = Self {
            capture,
            catalog: catalog.clone(),
            budget: Arc::new(Semaphore::new(2)),
            cancellations: std::array::from_fn(|_| Cancellation::default()),
            outbox: outbox.clone(),
            snapshots,
            ready: ready.clone(),
        };
        tasks.spawn(async move {
            let discovered = tokio::task::spawn_blocking(|| {
                let executable =
                    midnight_plugin_host::installed_executable().map_err(std::io::Error::other)?;
                Catalog::discover(executable.parent().unwrap_or(Path::new(".")))
            })
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
            let available = discovered.available_formats();
            let _ = catalog.set(discovered);
            ready.notify_waiters();
            outbox.send(Event::ArchiveAvailability(available)).await;
        });
        result
    }
    pub(crate) fn cancel(&mut self, pane: PaneId) {
        self.cancellations[pane.index()].cancel();
        let mut snapshots = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
        // A failed native navigation can still display the previous virtual snapshot.
        snapshots[pane.index()].archive_navigation = None;
    }
    pub(crate) fn navigate(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        action: ArchiveAction,
        sort: SortSpec,
        tasks: &mut JoinSet<()>,
    ) {
        self.cancellations[pane.index()].cancel();
        let cancel = Cancellation::default();
        self.cancellations[pane.index()] = cancel.clone();
        let catalog = self.catalog.clone();
        let capture = self.capture.clone();
        let ready = self.ready.clone();
        let snapshots = self.snapshots.clone();
        {
            let mut states = snapshots.lock().unwrap_or_else(|e| e.into_inner());
            states[pane.index()].archive_navigation = Some(navigation);
            states[pane.index()].pending = None;
        }
        let outbox = self.outbox.clone();
        let permit = self.budget.clone().try_acquire_owned();
        let restore_native_on_failure = matches!(&action, ArchiveAction::Open { .. });
        tasks.spawn(async move {
            loop {
                let notified = ready.notified();
                if catalog.get().is_some() {
                    break;
                }
                notified.await;
            }
            let result = match permit {
                Ok(permit) => tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let location = match action {
                        ArchiveAction::Open { path } => {
                            let (package, format) =
                                catalog.get().and_then(|c| c.choose(&path)).ok_or_else(|| {
                                    FsError::from_io(
                                        "open archive",
                                        &path,
                                        std::io::Error::new(
                                            std::io::ErrorKind::Unsupported,
                                            "Compatible archive filesystem plugin is unavailable",
                                        ),
                                    )
                                })?;
                            let source = capture_source(capture.as_ref(), &path, None, &cancel)?;
                            let mount =
                                Mount::open_captured(package, format, &path, source, &cancel)?;
                            ArchiveLocation {
                                address: mount.address(),
                                mount: Some(mount),
                                directory: 1,
                                trail: vec![(
                                    1,
                                    path.file_name()
                                        .unwrap_or(path.as_os_str())
                                        .to_string_lossy()
                                        .into_owned(),
                                )],
                            }
                        }
                        ArchiveAction::List(location) if location.mount.is_none() => {
                            restore(catalog.get(), capture.as_ref(), location.address, &cancel)?
                        }
                        ArchiveAction::List(location) => location,
                    };
                    let nodes = location
                        .mount
                        .as_ref()
                        .ok_or_else(|| error("Archive is unavailable"))?
                        .list(location.directory, &cancel)?;
                    let mut entries: Vec<_> = nodes.into_iter().map(Entry::from_archive).collect();
                    sort_entries(&mut entries, sort);
                    let index = Arc::new(DirectoryIndex::from_entries(&entries));
                    Ok(ArchiveListing {
                        location,
                        entries: entries.into(),
                        index,
                    })
                })
                .await
                .unwrap_or_else(|e| Err(error(e.to_string()))),
                Err(_) => Err(error("Archive navigation workers are busy")),
            };
            {
                let mut states = snapshots.lock().unwrap_or_else(|e| e.into_inner());
                let state = &mut states[pane.index()];
                if state.archive_navigation == Some(navigation) {
                    if let Ok(listing) = &result {
                        state.archive = Some((navigation, listing.entries.clone()));
                        state.committed = None;
                        state.visible = None;
                        state.pending = None;
                    } else if restore_native_on_failure && state.archive.is_none() {
                        // Opening a native archive failed: its original native pane remains.
                        // Saved provider tabs remain unavailable, with native actions disabled.
                        state.archive_navigation = None;
                    }
                }
            }
            outbox
                .send(Event::ArchiveListed {
                    pane,
                    navigation,
                    result,
                })
                .await;
        });
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        for cancel in &self.cancellations {
            cancel.cancel();
        }
    }
}
fn error(message: impl Into<String>) -> FsError {
    FsError::from_io(
        "archive navigation",
        Path::new(""),
        std::io::Error::other(message.into()),
    )
}

fn restore(
    catalog: Option<&Catalog>,
    capture: Option<&ArchiveCapture>,
    mut address: ArchiveAddress,
    cancel: &Cancellation,
) -> Result<ArchiveLocation, FsError> {
    address.validate().map_err(error)?;
    let (package, format) = catalog
        .and_then(|c| c.choose_provider(&address.container, Some(&address.provider)))
        .ok_or_else(|| {
            error("Saved archive filesystem plugin is unavailable; the tab address is retained")
        })?;
    let source = capture_source(
        capture,
        &address.container,
        Some(&address.source_version),
        cancel,
    )?;
    let mount = Mount::open_captured(package, format, &address.container, source, cancel)?;
    // The authority validated the saved token, including complete supported legacy encodings.
    address.source_version = mount.address().source_version;
    let mut location = ArchiveLocation {
        mount: Some(mount.clone()),
        directory: 1,
        trail: vec![(
            1,
            address
                .container
                .file_name()
                .unwrap_or(address.container.as_os_str())
                .to_string_lossy()
                .into_owned(),
        )],
        address,
    };
    for component in &location.address.components {
        let mut matches = mount
            .list(location.directory, cancel)?
            .into_iter()
            .filter(|node| {
                node.kind == EntryKind::Directory && node.raw_name.as_ref() == component.as_slice()
            });
        let node = matches
            .next()
            .ok_or_else(|| error("Saved archive folder is unavailable"))?;
        if matches.next().is_some() {
            return Err(error("Saved archive folder locator is ambiguous"));
        }
        location.directory = node.id.entry;
        location.trail.push((node.id.entry, node.display_name));
    }
    Ok(location)
}
