//! One coalescing two-pane comparison; stale work cannot publish selections, and
//! every request the UI still waits for is answered (a result, or why there is none).
use crate::{Outbox, Snapshots, task_lifetime::TaskLifetime};
use midnight_core::*;
use std::{io, sync::Arc};
use tokio::{sync::watch, task::JoinSet};
mod compute;
mod contents;
mod rules;
mod walk;

#[derive(Clone, Default)]
struct Intent {
    id: CompareId,
    serial: u64,
    request: Option<Arc<CompareRequest>>,
    /// A panel was navigated or re-read under the request `id`: it is answered with
    /// an explanation instead of a result.
    invalidated: bool,
}

pub(crate) struct Controller {
    input: watch::Sender<Intent>,
}

/// Native access for Compare contents and Include subfolders.
#[derive(Clone, Default)]
pub(crate) struct Services {
    pub(crate) find: Option<Arc<dyn FindService>>,
    /// Lists compared folders from their records, without opening every child.
    pub(crate) transfer: Option<Arc<dyn TransferService>>,
    /// Anchors the services while a blocking comparison still uses them.
    pub(crate) scope: Option<TaskLifetime>,
}

/// Stops native reads when the comparison task ends for any reason.
struct CancelOnDrop(Cancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl Controller {
    #[cfg(test)]
    pub(crate) fn new(snapshots: Snapshots, outbox: Outbox, tasks: &mut JoinSet<()>) -> Self {
        Self::with_services(snapshots, outbox, Services::default(), tasks)
    }

    pub(crate) fn with_services(
        snapshots: Snapshots,
        outbox: Outbox,
        services: Services,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let (input, receiver) = watch::channel(Intent::default());
        tasks.spawn(run(snapshots, outbox, services, receiver));
        Self { input }
    }

    pub(crate) fn compare(&self, request: CompareRequest) {
        self.input.send_if_modified(|intent| {
            if request.id <= intent.id {
                return false;
            }
            intent.id = request.id;
            intent.serial = intent.serial.wrapping_add(1);
            intent.request = Some(Arc::new(request));
            intent.invalidated = false;
            true
        });
    }

    pub(crate) fn cancel(&self, id: CompareId) {
        self.input.send_if_modified(|intent| {
            if id < intent.id {
                return false;
            }
            intent.id = id;
            intent.serial = intent.serial.wrapping_add(1);
            intent.request = None;
            intent.invalidated = false;
            true
        });
    }

    pub(crate) fn invalidate(&self, _pane: PaneId) {
        self.input.send_modify(|intent| {
            intent.serial = intent.serial.wrapping_add(1);
            intent.invalidated |= intent.request.take().is_some();
        });
    }
}

fn source_matches(snapshots: &Snapshots, request: &CompareRequest) -> bool {
    let snapshots = snapshots.lock().unwrap_or_else(|error| error.into_inner());
    request.sides.iter().enumerate().all(|(slot, side)| {
        let state = &snapshots[slot];
        side.source.pane.index() == slot
            && state.pending.is_none()
            && state.has_native_view()
            && side.tree.as_ref().is_none_or(|tree| {
                tree.stamp.pane.index() == slot
                    && tree.stamp.navigation == side.source.navigation
                    && Arc::ptr_eq(&tree.root.entries, &side.source.entries)
                    && Arc::ptr_eq(&tree.index.directory, &side.view)
                    && state.capture_tree_root(&tree.root).is_ok()
            })
            && [&state.visible, &state.committed]
                .into_iter()
                .flatten()
                .any(|view| {
                    view.navigation == side.source.navigation
                        && view.path == side.source.path
                        && Arc::ptr_eq(&view.entries, &side.source.entries)
                })
    })
}

/// Whether the UI still waits for an answer to `request`: no newer request and no
/// Cancel came. A panel change (`invalidated`) still owes one.
fn awaited(latest: &Intent, request: &CompareRequest) -> bool {
    latest.id == request.id && (latest.request.is_some() || latest.invalidated)
}

const PANELS_CHANGED: &str =
    "A panel was refreshed or opened another folder while the comparison ran. Compare again.";

async fn run(
    snapshots: Snapshots,
    outbox: Outbox,
    services: Services,
    mut input: watch::Receiver<Intent>,
) {
    let mut pending = false;
    loop {
        if !pending && input.changed().await.is_err() {
            return;
        }
        pending = false;
        let intent = input.borrow_and_update().clone();
        let Some(request) = intent.request else {
            continue;
        };
        let valid = source_matches(&snapshots, &request);
        let result = if valid {
            let token = Cancellation::default();
            let _stop = CancelOnDrop(token.clone());
            let work = request.clone();
            let observer = input.clone();
            let find = services.find.clone();
            let transfer = services.transfer.clone();
            let progress = outbox.clone();
            let worker_token = token.clone();
            let job = move || {
                let id = work.id;
                let mut report = |sample: &CompareProgress| {
                    if worker_token.is_cancelled() {
                        return;
                    }
                    // Samples are disposable: a full queue skips one.
                    if progress
                        .sender
                        .try_send(Event::ComparisonProgress {
                            id,
                            progress: sample.clone(),
                        })
                        .is_ok()
                    {
                        (progress.wakeup)();
                    }
                };
                let deep = find.as_deref().map(|find| compute::Deep {
                    find,
                    transfer: transfer.as_deref(),
                    cancel: &worker_token,
                    report: &mut report,
                });
                let cancelled =
                    || worker_token.is_cancelled() || observer.has_changed().unwrap_or(true);
                compute::compare(work, &cancelled, deep)
            };
            let mut worker = match &services.scope {
                Some(scope) => tokio::task::spawn_blocking(scope.blocking(job)),
                None => tokio::task::spawn_blocking(job),
            };
            let mut watching = true;
            let mut closed = false;
            let joined = loop {
                tokio::select! {
                    joined = &mut worker => break joined,
                    changed = input.changed(), if watching => {
                        // A newer request, Cancel or a panel change: stop native reads now.
                        token.cancel();
                        watching = false;
                        match changed {
                            Ok(()) => pending = true,
                            Err(_) => closed = true,
                        }
                    }
                }
            };
            if closed {
                return;
            }
            joined.unwrap_or_else(|error| Err(failure(&request, error.to_string())))
        } else {
            Err(failure(&request, PANELS_CHANGED))
        };
        // Answer every request the UI still waits for. Before, a request whose panels
        // changed while it ran was dropped without an answer, and the UI waited for it
        // for ever: "Comparing subfolders…" stayed and every refresh stayed blocked.
        let mut event = Some(Event::DirectoriesCompared {
            id: request.id,
            result: result.map(Arc::new),
        });
        while let Some(answer) = event.take() {
            tokio::select! {
                biased;
                changed = input.changed() => {
                    if changed.is_err() { return; }
                    pending = true;
                    if awaited(&input.borrow(), &request) {
                        event = Some(answer);
                    }
                }
                permit = outbox.sender.reserve() => {
                    let Ok(permit) = permit else { return; };
                    let answer = {
                        let latest = input.borrow();
                        if !awaited(&latest, &request) {
                            None
                        } else if latest.invalidated
                            || latest.serial != intent.serial
                            || (valid && !source_matches(&snapshots, &request))
                        {
                            Some(Event::DirectoriesCompared {
                                id: request.id,
                                result: Err(failure(&request, PANELS_CHANGED)),
                            })
                        } else {
                            Some(answer)
                        }
                    };
                    if let Some(answer) = answer {
                        permit.send(answer);
                        (outbox.wakeup)();
                    }
                }
            }
        }
    }
}

fn failure(request: &CompareRequest, message: impl Into<String>) -> FsError {
    FsError::from_io(
        "compare directories",
        &request.sides[0].source.path,
        io::Error::new(io::ErrorKind::InvalidData, message.into()),
    )
}

#[cfg(test)]
fn compute(
    request: Arc<CompareRequest>,
    cancelled: &dyn Fn() -> bool,
) -> Result<CompareResult, FsError> {
    compute::compare(request, cancelled, None)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tree_tests;

#[cfg(test)]
mod deep_tests;

#[cfg(test)]
mod native_bench_tests;
