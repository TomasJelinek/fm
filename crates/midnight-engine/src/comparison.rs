//! One coalescing two-pane comparison; stale work cannot publish selections.
use crate::{Outbox, Snapshots};
use midnight_core::*;
use std::{io, sync::Arc};
use tokio::{sync::watch, task::JoinSet};
mod compute;

#[derive(Clone, Default)]
struct Intent {
    id: CompareId,
    serial: u64,
    request: Option<Arc<CompareRequest>>,
}

pub(crate) struct Controller {
    input: watch::Sender<Intent>,
}

impl Controller {
    pub(crate) fn new(snapshots: Snapshots, outbox: Outbox, tasks: &mut JoinSet<()>) -> Self {
        let (input, receiver) = watch::channel(Intent::default());
        tasks.spawn(run(snapshots, outbox, receiver));
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
            true
        });
    }

    pub(crate) fn invalidate(&self, _pane: PaneId) {
        self.input.send_modify(|intent| {
            intent.serial = intent.serial.wrapping_add(1);
            intent.request = None;
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

async fn run(snapshots: Snapshots, outbox: Outbox, mut input: watch::Receiver<Intent>) {
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
            let work = request.clone();
            let observer = input.clone();
            tokio::task::spawn_blocking(move || {
                compute(work, &|| observer.has_changed().unwrap_or(true))
            })
            .await
            .unwrap_or_else(|error| Err(failure(&request, error.to_string())))
        } else {
            Err(failure(
                &request,
                "The captured directories are no longer current.",
            ))
        };
        if result
            .as_ref()
            .is_err_and(|error| error.kind == io::ErrorKind::Interrupted)
        {
            continue;
        }
        let event = Event::DirectoriesCompared {
            id: request.id,
            result: result.map(Arc::new),
        };
        tokio::select! {
            biased;
            changed = input.changed() => {
                if changed.is_err() { return; }
                pending = true;
            }
            permit = outbox.sender.reserve() => {
                let Ok(permit) = permit else { return; };
                let sent = {
                    let latest = input.borrow();
                    if latest.serial != intent.serial || (valid && !source_matches(&snapshots, &request)) {
                        false
                    } else { permit.send(event); true }
                };
                if sent { (outbox.wakeup)(); }
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

fn compute(
    request: Arc<CompareRequest>,
    cancelled: &dyn Fn() -> bool,
) -> Result<CompareResult, FsError> {
    compute::compare(request, cancelled)
}

#[cfg(test)]
mod tests;
