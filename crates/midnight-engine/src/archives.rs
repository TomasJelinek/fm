//! Bounded archive navigation and canonical guarded native mount ownership.
use crate::{Outbox, Snapshots, retirement::WorkerOwned, task_lifetime::TaskLifetime};
use midnight_core::*;
use midnight_filesystem_host::{Catalog, Mount, NativeAllowance, NativeSlot};
use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::{
    sync::{Notify, OwnedSemaphorePermit, Semaphore},
    task::JoinSet,
};
mod native;
mod navigation;
mod ownership;
use ownership::Mounts;
pub(crate) use ownership::{RETIREMENT_CAPACITY, Retirement};
#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod retirement_tests;
#[cfg(test)]
mod root_retirement_tests;
#[cfg(test)]
mod size_sort_tests;
#[cfg(test)]
mod tests;
mod timestamps;
mod unlock;

const PANES: usize = 2;
const MAX_WORKERS: usize = 2;
/// Accepted navigations alive at once, each holding one share and one retirement
/// slot: one per worker and the newest navigation of each pane waiting for a
/// worker. A superseded input keeps its share until the cleanup worker destroys
/// it, so the intake waits for that instead of spending cleanup slots that belong
/// to other engine owners.
const MAX_REQUESTS: usize = MAX_WORKERS + PANES;
/// Discovery diagnostics carried into an "unavailable" navigation error.
const MAX_REPORTED_DIAGNOSTICS: usize = 4;
const MAX_DIAGNOSTIC_CHARS: usize = 512;
/// How long a navigation waits for the session it replaces to end (`native::slot`). A
/// session that its holders let go ends well within it; one still held never does.
const HANDOFF_DEADLINE: Duration = Duration::from_secs(10);

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
    /// The slot reserved for the archive that this navigation opens from a native file;
    /// its session takes the slot, and a failed navigation releases it.
    native: Mutex<Option<NativeSlot>>,
    opener: native::Opener,
    _scope: TaskLifetime,
    // Last: native input and original service owners finish cleanup first.
    permit: Mutex<Option<OwnedSemaphorePermit>>,
    /// One of the MAX_REQUESTS shares, returned after everything above.
    _share: OwnedSemaphorePermit,
}
pub(crate) struct Controller {
    capture: Option<ArchiveCapture>,
    locale: Option<Arc<dyn LocaleService>>,
    catalog: Arc<OnceLock<Catalog>>,
    budget: Arc<Semaphore>,
    /// MAX_REQUESTS navigation shares; see [`Controller::admission`].
    requests: Arc<Semaphore>,
    cancellations: [Cancellation; PANES],
    /// Wakes navigations waiting for a worker after a cancellation.
    superseded: Arc<Notify>,
    outbox: Outbox,
    snapshots: Snapshots,
    ready: Arc<Notify>,
    mounts: Arc<Mounts>,
    retirement: Retirement,
    scope: TaskLifetime,
    /// The slots of archives opened from native files (`NativeAllowance::global`).
    allowance: Arc<NativeAllowance>,
    opener: native::Opener,
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
            requests: Arc::new(Semaphore::new(MAX_REQUESTS)),
            cancellations: std::array::from_fn(|_| Cancellation::default()),
            superseded: Arc::new(Notify::new()),
            outbox: outbox.clone(),
            snapshots,
            ready: ready.clone(),
            mounts: Arc::new(Mounts::new(retirement.domain.clone())),
            retirement,
            scope: scope.clone(),
            allowance: NativeAllowance::global(),
            opener: native::Opener::Installed,
        };
        let native_scope = scope.clone();
        tasks.spawn(scope.future(async move {
            let discovered = match tokio::task::spawn_blocking(native_scope.blocking(|| {
                let executable =
                    midnight_plugin_host::installed_executable().map_err(std::io::Error::other)?;
                Catalog::discover(executable.parent().unwrap_or(Path::new(".")))
            }))
            .await
            {
                Ok(Ok(catalog)) => catalog,
                // Keep the cause: every later archive open reports it as unavailable.
                Ok(Err(error)) => failed_discovery(format!(
                    "Archive filesystem plugin discovery failed: {error}"
                )),
                Err(error) => failed_discovery(format!(
                    "Archive filesystem plugin discovery stopped: {error}"
                )),
            };
            for diagnostic in discovered.diagnostics.iter().take(MAX_REPORTED_DIAGNOSTICS) {
                eprintln!("Archive filesystem discovery: {diagnostic}");
            }
            let available = discovered.available_formats();
            let _ = catalog.set(discovered);
            ready.notify_waiters();
            // Check the archive spool root now, in the background: the check can wait
            // seconds for a busy disk, and the first archive opened would wait with it.
            let prepared = catalog.clone();
            drop(tokio::task::spawn_blocking(move || {
                if let Some(Err(error)) = prepared.get().map(Catalog::prepare_staging) {
                    eprintln!("Archive spool check: {error}");
                }
            }));
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
    /// One of the MAX_REQUESTS navigation shares, returned when the cleanup worker
    /// destroys that navigation's input. The intake first cancels its pane's
    /// previous navigation. When every share is then held, at most MAX_WORKERS
    /// holders run and the other pane has one current navigation, so at least one
    /// holder is a superseded navigation waiting for a worker: it hands its input
    /// to the cleanup worker without a navigation worker, this dispatcher or UI
    /// delivery, and the wait ends.
    async fn admission(&self) -> OwnedSemaphorePermit {
        self.requests
            .clone()
            .acquire_owned()
            .await
            .expect("archive navigation shares are never closed")
    }
    pub(crate) fn cancel(&mut self, pane: PaneId) {
        self.cancellations[pane.index()].cancel();
        self.superseded.notify_waiters();
        let mut snapshots = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
        // A failed native navigation can still display the previous virtual snapshot.
        snapshots[pane.index()].archive_navigation = None;
    }
    #[cfg(test)]
    pub(crate) async fn navigate(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        action: ArchiveAction,
        sort: SortSpec,
        tasks: &mut JoinSet<()>,
    ) {
        self.navigate_replacing(pane, navigation, action, sort, None, tasks)
            .await;
    }
    /// `replaces`: the session that this pane's tab let go of for this navigation
    /// (`Command::ArchiveNavigate`).
    pub(crate) async fn navigate_replacing(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        action: ArchiveAction,
        sort: SortSpec,
        replaces: Option<u64>,
        tasks: &mut JoinSet<()>,
    ) {
        // EngineRun protects the incoming command while this single intake waits.
        let lease = self.retirement.intake.reserve().await;
        self.cancellations[pane.index()].cancel();
        self.superseded.notify_waiters();
        let cancel = Cancellation::default();
        self.cancellations[pane.index()] = cancel.clone();
        let restore_native_on_failure = matches!(&action, ArchiveAction::Open { .. });
        let target = native::Target::of(&action);
        {
            // Before any wait: a superseded result must not act as the current one.
            let mut states = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
            states[pane.index()].archive_navigation = Some(navigation);
            states[pane.index()].pending = None;
        }
        let share = self.admission().await;
        let request = lease.own(Request {
            action: Mutex::new(Some(action)),
            capture: self.capture.clone(),
            locale: self.locale.clone(),
            catalog: self.catalog.clone(),
            snapshots: self.snapshots.clone(),
            pane,
            navigation,
            sort,
            cancel,
            native: Mutex::new(None),
            opener: self.opener.clone(),
            _scope: self.scope.clone(),
            permit: Mutex::new(None),
            _share: share,
        });
        let request: Arc<WorkerOwned<Request>> = Arc::new(request);
        let ready = self.ready.clone();
        let outbox = self.outbox.clone();
        let mounts = self.mounts.clone();
        let scope = self.scope.clone();
        let budget = self.budget.clone();
        let superseded = self.superseded.clone();
        let allowance = self.allowance.clone();
        tasks.spawn(self.scope.future(async move {
            // Superseded listings keep their worker until the native call returns.
            // The newest navigation waits for one instead of being refused; each
            // pane has at most one such waiter because a newer request cancels it.
            let Some(permit) = admit(&budget, &request.cancel, &superseded).await else {
                // Hand the input and its share to cleanup before delivery, which
                // can wait for the UI while the intake waits for this share.
                let snapshots = request.snapshots.clone();
                drop(request);
                navigation::publish(
                    &snapshots,
                    &outbox,
                    pane,
                    navigation,
                    restore_native_on_failure,
                    Err(FsError::from_io(
                        "archive navigation",
                        Path::new(""),
                        std::io::ErrorKind::Interrupted.into(),
                    )),
                )
                .await;
                return;
            };
            *request.permit.lock().unwrap_or_else(|e| e.into_inner()) = Some(permit);
            loop {
                let notified = ready.notified();
                if request.catalog.get().is_some() {
                    break;
                }
                notified.await;
            }
            if let Some(target) = &target
                && let Err(error) =
                    native::slot(&request, target, replaces, &allowance, &superseded, &scope).await
            {
                let snapshots = request.snapshots.clone();
                drop(request);
                navigation::publish(
                    &snapshots,
                    &outbox,
                    pane,
                    navigation,
                    restore_native_on_failure,
                    Err(error),
                )
                .await;
                return;
            }
            let worker = request.clone();
            let progress = outbox.clone();
            let result = tokio::task::spawn_blocking(
                scope.blocking(move || navigation::run(&worker, &mounts, &progress)),
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
            requests: Arc::new(Semaphore::new(MAX_REQUESTS)),
            cancellations: std::array::from_fn(|_| Cancellation::default()),
            superseded: Arc::new(Notify::new()),
            outbox,
            snapshots,
            ready: Arc::new(Notify::new()),
            mounts: Arc::new(Mounts::new(retirement.domain.clone())),
            retirement,
            scope: TaskLifetime::testing(()),
            allowance: NativeAllowance::new(2),
            opener: native::Opener::Installed,
        }
    }
}

impl Request {
    /// The slot reserved for this navigation's new session (`native::slot`), if any.
    fn take_native_slot(&self) -> Option<NativeSlot> {
        self.native.lock().unwrap_or_else(|e| e.into_inner()).take()
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
        self.superseded.notify_waiters();
    }
}

/// Wait for a navigation worker, or return None once this request is cancelled.
async fn admit(
    budget: &Arc<Semaphore>,
    cancel: &Cancellation,
    superseded: &Notify,
) -> Option<OwnedSemaphorePermit> {
    // One acquisition keeps its FIFO place while other cancellations wake this waiter.
    let acquire = budget.clone().acquire_owned();
    tokio::pin!(acquire);
    loop {
        // Register before checking so a concurrent cancellation cannot be missed.
        let changed = superseded.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if cancel.is_cancelled() {
            return None;
        }
        tokio::select! {
            permit = &mut acquire => return permit.ok(),
            () = &mut changed => {}
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

fn failed_discovery(diagnostic: String) -> Catalog {
    Catalog {
        diagnostics: vec![diagnostic],
        ..Catalog::default()
    }
}

/// An unavailable-provider error that keeps the bounded discovery diagnostics
/// explaining why no compatible package was loaded.
fn unavailable(message: &str, catalog: Option<&Catalog>) -> String {
    let mut text = message.to_owned();
    let diagnostics = catalog.map_or(&[][..], |catalog| catalog.diagnostics.as_slice());
    for diagnostic in diagnostics.iter().take(MAX_REPORTED_DIAGNOSTICS) {
        text.push_str("\nPlugin discovery: ");
        text.extend(diagnostic.chars().take(MAX_DIAGNOSTIC_CHARS));
    }
    if diagnostics.len() > MAX_REPORTED_DIAGNOSTICS {
        text.push_str(&format!(
            "\n{} more plugin discovery diagnostics",
            diagnostics.len() - MAX_REPORTED_DIAGNOSTICS
        ));
    }
    text
}

fn restore(
    request: &Request,
    slot: Option<NativeSlot>,
    mut address: ArchiveAddress,
) -> Result<ArchiveLocation, FsError> {
    let cancel = &request.cancel;
    address.validate().map_err(error)?;
    let (mount, version) = request.opener.restore(request, slot, &address)?;
    // The capture authority validated the saved token, including supported legacy encodings.
    address.source_version = version;
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
