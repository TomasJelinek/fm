//! Bounded archive navigation and canonical guarded native mount ownership.
use crate::{Outbox, Snapshots, retirement::WorkerOwned, task_lifetime::TaskLifetime};
use midnight_core::*;
use midnight_filesystem_host::{Catalog, Mount};
use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::{
    sync::{Notify, OwnedSemaphorePermit, Semaphore},
    task::JoinSet,
};
mod navigation;
mod ownership;
use ownership::Mounts;
pub(crate) use ownership::{RETIREMENT_CAPACITY, Retirement};
#[cfg(test)]
mod retirement_tests;
#[cfg(test)]
mod root_retirement_tests;
#[cfg(test)]
mod size_sort_tests;
#[cfg(test)]
mod tests;
mod timestamps;

const PANES: usize = 2;
const MAX_WORKERS: usize = 2;

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

struct Request {
    action: Mutex<Option<ArchiveAction>>,
    capture: Option<ArchiveCapture>,
    locale: Option<Arc<dyn LocaleService>>,
    catalog: Arc<OnceLock<Catalog>>,
    snapshots: Snapshots,
    pane: PaneId,
    navigation: NavigationId,
    sort: SortSpec,
    cancel: Cancellation,
    scope: TaskLifetime,
    // Last: native input and original service owners finish cleanup first.
    permit: Option<OwnedSemaphorePermit>,
}
pub(crate) struct Controller {
    capture: Option<ArchiveCapture>,
    locale: Option<Arc<dyn LocaleService>>,
    catalog: Arc<OnceLock<Catalog>>,
    budget: Arc<Semaphore>,
    cancellations: [Cancellation; PANES],
    outbox: Outbox,
    snapshots: Snapshots,
    ready: Arc<Notify>,
    mounts: Arc<Mounts>,
    retirement: Retirement,
    scope: TaskLifetime,
}
impl Controller {
    pub(crate) fn new(
        retirement: Retirement,
        scope: TaskLifetime,
        capture: Option<ArchiveCapture>,
        locale: Option<Arc<dyn LocaleService>>,
        outbox: Outbox,
        snapshots: Snapshots,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let catalog = Arc::new(OnceLock::new());
        let ready = Arc::new(Notify::new());
        let result = Self {
            capture,
            locale,
            catalog: catalog.clone(),
            budget: Arc::new(Semaphore::new(MAX_WORKERS)),
            cancellations: std::array::from_fn(|_| Cancellation::default()),
            outbox: outbox.clone(),
            snapshots,
            ready: ready.clone(),
            mounts: Arc::new(Mounts::new(retirement.domain.clone())),
            retirement,
            scope: scope.clone(),
        };
        let native_scope = scope.clone();
        tasks.spawn(scope.future(async move {
            let discovered = tokio::task::spawn_blocking(native_scope.blocking(|| {
                let executable =
                    midnight_plugin_host::installed_executable().map_err(std::io::Error::other)?;
                Catalog::discover(executable.parent().unwrap_or(Path::new(".")))
            }))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
            let available = discovered.available_formats();
            let _ = catalog.set(discovered);
            ready.notify_waiters();
            outbox.send(Event::ArchiveAvailability(available)).await;
        }));
        result
    }
    /// Serial dispatcher rejection before normal navigation admission.
    /// EngineRun owns the raw input while the payload-free reservation is pending.
    pub(crate) async fn retire_action(
        &mut self,
        action: ArchiveAction,
    ) -> Result<(), crate::retirement::RetirementPanic> {
        let lease = self.retirement.intake.reserve().await;
        lease.own((action, self.scope.clone())).retire().await
    }
    pub(crate) fn cancel(&mut self, pane: PaneId) {
        self.cancellations[pane.index()].cancel();
        let mut snapshots = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
        // A failed native navigation can still display the previous virtual snapshot.
        snapshots[pane.index()].archive_navigation = None;
    }
    pub(crate) async fn navigate(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        action: ArchiveAction,
        sort: SortSpec,
        tasks: &mut JoinSet<()>,
    ) {
        // EngineRun protects the incoming command while this single intake waits.
        let lease = self.retirement.intake.reserve().await;
        self.cancellations[pane.index()].cancel();
        let cancel = Cancellation::default();
        self.cancellations[pane.index()] = cancel.clone();
        let restore_native_on_failure = matches!(&action, ArchiveAction::Open { .. });
        let mut request = lease.own(Request {
            action: Mutex::new(Some(action)),
            capture: self.capture.clone(),
            locale: self.locale.clone(),
            catalog: self.catalog.clone(),
            snapshots: self.snapshots.clone(),
            pane,
            navigation,
            sort,
            cancel,
            scope: self.scope.clone(),
            permit: None,
        });
        {
            let mut states = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
            states[pane.index()].archive_navigation = Some(navigation);
            states[pane.index()].pending = None;
        }
        let Ok(permit) = self.budget.clone().try_acquire_owned() else {
            let failure = match request.retire().await {
                Ok(()) => error("Archive navigation workers are busy"),
                Err(failure) => error(format!("Archive input cleanup failed: {failure}")),
            };
            navigation::publish(
                &self.snapshots,
                &self.outbox,
                pane,
                navigation,
                restore_native_on_failure,
                Err(failure),
            )
            .await;
            return;
        };
        request.permit = Some(permit);
        let request: Arc<WorkerOwned<Request>> = Arc::new(request);
        let ready = self.ready.clone();
        let outbox = self.outbox.clone();
        let mounts = self.mounts.clone();
        let scope = self.scope.clone();
        tasks.spawn(self.scope.future(async move {
            loop {
                let notified = ready.notified();
                if request.catalog.get().is_some() {
                    break;
                }
                notified.await;
            }
            let worker = request.clone();
            let result = tokio::task::spawn_blocking(
                scope.blocking(move || navigation::run(&worker, &mounts)),
            )
            .await
            .unwrap_or_else(|e| Err(error(e.to_string())));
            navigation::publish(
                &request.snapshots,
                &outbox,
                pane,
                navigation,
                restore_native_on_failure,
                result,
            )
            .await;
        }));
    }
    #[cfg(test)]
    fn testing(outbox: Outbox, snapshots: Snapshots, budget: Arc<Semaphore>) -> Self {
        let catalog = OnceLock::new();
        catalog.set(Catalog::default()).unwrap();
        let retirement = Retirement::testing();
        Self {
            capture: None,
            locale: None,
            catalog: Arc::new(catalog),
            budget,
            cancellations: std::array::from_fn(|_| Cancellation::default()),
            outbox,
            snapshots,
            ready: Arc::new(Notify::new()),
            mounts: Arc::new(Mounts::new(retirement.domain.clone())),
            retirement,
            scope: TaskLifetime::testing(()),
        }
    }
}

/// Called inside the admitted blocking listing worker, which retains its permit
/// until all displaced source handles have been retired outside the state lock.
fn remember_listing(
    snapshots: &Snapshots,
    pane: PaneId,
    navigation: NavigationId,
    listing: &ArchiveListing,
    cancel: &Cancellation,
) {
    let retired = {
        let mut states = snapshots.lock().unwrap_or_else(|error| error.into_inner());
        let state = &mut states[pane.index()];
        if cancel.is_cancelled() || state.archive_navigation != Some(navigation) {
            return;
        }
        let retired = state.remember_archive_tree_root(pane, navigation, listing);
        state.archive = Some((navigation, listing.entries.clone()));
        state.committed = None;
        state.visible = None;
        state.pending = None;
        retired
    };
    drop(retired);
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
        .and_then(|c| match address.format {
            Some(format) => c.choose_format(format, Some(&address.provider)),
            None => c.choose_provider(&address.container, Some(&address.provider)),
        })
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
    // The capture authority validated the saved token, including supported legacy encodings.
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
