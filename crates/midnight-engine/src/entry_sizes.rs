//! Independent Space-key directory requests share global scan budgets without cancelling peers.
use crate::{
    CommittedSnapshot, Outbox, SharedSizes, Snapshots, folder_sizes, task_lifetime::TaskLifetime,
};
use midnight_core::{
    Cancellation, EntryId, EntryKind, Event, FileSystem, FolderSizeScanId, NavigationId, PaneId,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::Semaphore,
    task::{AbortHandle, JoinSet},
};

const MAX_PENDING_PER_PANE: usize = 64;

struct Request {
    navigation: NavigationId,
    scan: FolderSizeScanId,
    entry: EntryId,
    path: PathBuf,
    cancel: Cancellation,
    finished: Arc<AtomicBool>,
    task: AbortHandle,
}

impl Request {
    fn cancel(self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

pub(crate) struct Services {
    pub(crate) filesystem: Arc<dyn FileSystem>,
    pub(crate) scope: TaskLifetime,
    pub(crate) reports: crate::ReportArena,
}

pub(crate) struct Controller {
    filesystem: Arc<dyn FileSystem>,
    outbox: Outbox,
    snapshots: Snapshots,
    budget: Arc<Semaphore>,
    delivery_cancel: Cancellation,
    io_budget: Arc<folder_sizes::IoBudget>,
    size_observer: crate::selection_sizes::Observer,
    pending: [Vec<Request>; 2],
    scope: TaskLifetime,
    reports: crate::ReportArena,
}

impl Controller {
    pub(crate) fn new(
        services: Services,
        outbox: Outbox,
        snapshots: Snapshots,
        budget: Arc<Semaphore>,
        delivery_cancel: Cancellation,
        io_budget: Arc<folder_sizes::IoBudget>,
        size_observer: crate::selection_sizes::Observer,
    ) -> Self {
        let Services {
            filesystem,
            scope,
            reports,
        } = services;
        Self {
            filesystem,
            outbox,
            snapshots,
            budget,
            delivery_cancel,
            io_budget,
            size_observer,
            pending: Default::default(),
            scope,
            reports,
        }
    }

    pub(crate) fn compute(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        scan: FolderSizeScanId,
        entry: EntryId,
        tasks: &mut JoinSet<()>,
    ) {
        self.pending[pane.index()].retain(|request| !request.finished.load(Ordering::Acquire));
        if self.pending[pane.index()]
            .iter()
            .any(|request| request.scan == scan)
        {
            return;
        }
        let snapshot = {
            let snapshots = self
                .snapshots
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let state = &snapshots[pane.index()];
            state
                .visible
                .as_ref()
                .filter(|snapshot| snapshot.navigation == navigation)
                .or_else(|| {
                    state
                        .committed
                        .as_ref()
                        .filter(|snapshot| snapshot.navigation == navigation)
                })
                .filter(|_| state.pending.is_none() && state.has_native_view())
                .cloned()
        };
        let duplicate = self.pending[pane.index()]
            .iter()
            .any(|request| request.navigation == navigation && request.entry == entry);
        let Some(snapshot) = snapshot
            .filter(|_| !duplicate && self.pending[pane.index()].len() < MAX_PENDING_PER_PANE)
        else {
            self.finished(pane, navigation, scan, true, tasks);
            return;
        };
        let cancel = Cancellation::default();
        let worker_cancel = cancel.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let done = finished.clone();
        let filesystem = self.filesystem.clone();
        let reports = self.reports.clone();
        let outbox = self.outbox.clone();
        let snapshots = self.snapshots.clone();
        let budget = self.budget.clone();
        let delivery_cancel = self.delivery_cancel.clone();
        let io_budget = self.io_budget.clone();
        let size_observer = self.size_observer.clone();
        let path = snapshot.path.clone();
        let scope = self.scope.clone();
        let task = tasks.spawn(self.scope.future(async move {
            let Ok(permit) = budget.acquire_owned().await else {
                return;
            };
            let final_cancel = worker_cancel.clone();
            let final_outbox = outbox.clone();
            let result = tokio::task::spawn_blocking(scope.blocking(move || {
                let _permit = permit;
                if worker_cancel.is_cancelled() {
                    return false;
                }
                let Some(root) = snapshot.entries.iter().find(|row| {
                    row.id == entry
                        && matches!(row.kind, EntryKind::Directory | EntryKind::DirectoryLink)
                }) else {
                    return false;
                };
                folder_sizes::compute_batched(
                    filesystem.as_ref(),
                    std::slice::from_ref(root),
                    &worker_cancel,
                    &mut |entry, size| {
                        record_size(
                            &snapshot,
                            &snapshots,
                            pane,
                            &root.path,
                            &size,
                            &worker_cancel,
                        );
                        if let Some(event) =
                            size_observer.updated(pane, &snapshot.sizes, &root.path)
                        {
                            size_observer.publish(event);
                        }
                        outbox.blocking_send(
                            Event::FolderSizeUpdated {
                                pane,
                                navigation,
                                scan,
                                entry,
                                size,
                            },
                            &delivery_cancel,
                        )
                    },
                    &io_budget,
                    &reports,
                );
                !worker_cancel.is_cancelled()
            }))
            .await;
            done.store(true, Ordering::Release);
            final_outbox
                .send(Event::FolderSizesFinished {
                    pane,
                    navigation,
                    scan,
                    cancelled: final_cancel.is_cancelled() || !matches!(result, Ok(true)),
                })
                .await;
        }));
        self.pending[pane.index()].push(Request {
            navigation,
            scan,
            entry,
            path,
            cancel,
            finished,
            task,
        });
    }

    pub(crate) fn cancel(&mut self, pane: PaneId, scan: FolderSizeScanId, tasks: &mut JoinSet<()>) {
        if let Some(index) = self.pending[pane.index()]
            .iter()
            .position(|request| request.scan == scan)
        {
            let request = self.pending[pane.index()].remove(index);
            let navigation = request.navigation;
            request.cancel();
            self.finished(pane, navigation, scan, true, tasks);
        }
    }

    pub(crate) fn navigate(&mut self, pane: PaneId, path: &Path, same_context: bool) {
        let mut retained = Vec::new();
        for request in self.pending[pane.index()].drain(..) {
            if same_context && request.path == path {
                retained.push(request);
            } else {
                request.cancel();
            }
        }
        self.pending[pane.index()] = retained;
    }

    /// Pin the live cache before a same-location refresh starts. A target can finish
    /// while the new listing is blocked; both generations must still share its result.
    pub(crate) fn continued_sizes(
        &self,
        pane: PaneId,
        path: &Path,
        continued_scan: Option<FolderSizeScanId>,
    ) -> Option<SharedSizes> {
        if !self.pending[pane.index()].iter().any(|request| {
            request.path == path
                && (continued_scan == Some(request.scan)
                    || !request.finished.load(Ordering::Acquire))
        }) {
            return None;
        }
        let snapshots = self
            .snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let state = &snapshots[pane.index()];
        state
            .committed
            .as_ref()
            .filter(|snapshot| snapshot.path == path)
            .or_else(|| {
                state
                    .visible
                    .as_ref()
                    .filter(|snapshot| snapshot.path == path)
            })
            .map(|snapshot| snapshot.sizes.clone())
    }

    /// A newly accepted full-panel scan supersedes every earlier single-entry scan.
    pub(crate) fn cancel_all(&mut self, pane: PaneId) {
        for request in self.pending[pane.index()].drain(..) {
            request.cancel();
        }
    }

    fn finished(
        &self,
        pane: PaneId,
        navigation: NavigationId,
        scan: FolderSizeScanId,
        cancelled: bool,
        tasks: &mut JoinSet<()>,
    ) {
        let outbox = self.outbox.clone();
        tasks.spawn(async move {
            outbox
                .send(Event::FolderSizesFinished {
                    pane,
                    navigation,
                    scan,
                    cancelled,
                })
                .await;
        });
    }

    pub(crate) fn shutdown(mut self) {
        for pane in &mut self.pending {
            for request in pane.drain(..) {
                request.cancel();
            }
        }
    }
}

fn record_size(
    snapshot: &CommittedSnapshot,
    snapshots: &Snapshots,
    pane: PaneId,
    path: &Path,
    size: &midnight_core::FolderSize,
    cancel: &Cancellation,
) {
    snapshot
        .sizes
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(path.into(), size.clone());
    // Same-location refresh/resort must retain ongoing size estimates for later explicit sorting.
    let caches = {
        let state = snapshots.lock().unwrap_or_else(|error| error.into_inner());
        if cancel.is_cancelled() {
            return;
        }
        [&state[pane.index()].visible, &state[pane.index()].committed]
            .into_iter()
            .flatten()
            .filter(|current| current.path == snapshot.path)
            .map(|current| current.sizes.clone())
            .collect::<Vec<_>>()
    };
    for cache in caches {
        cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(path.into(), size.clone());
    }
}
