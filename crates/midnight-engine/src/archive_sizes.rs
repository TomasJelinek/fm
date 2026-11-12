//! Two bounded metadata-only archive size lanes; native scanning stays independent.
mod cache;
pub(crate) mod published;
pub(crate) mod scan;
mod worker;

use crate::{
    Outbox, Snapshots,
    retirement::{RetirementLease, WorkerOwned},
    tree_roots::ReadyRoot,
};
use midnight_core::{
    ArchiveSizeCommand as Command, ArchiveSizeEvent as Event, Cancellation, EntryId,
    FolderSizeScanId, PaneId, SelectionRevision, SelectionSizeId,
};
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Condvar, Mutex, Weak},
};
use tokio::task::JoinHandle;

const MAX_PENDING: usize = 8;

/// One shared pending owner per pane, reserved before any root is admitted.
pub(crate) const RETIREMENT_CAPACITY: usize = 2;
pub(crate) struct Retirement {
    pub(crate) panes: [RetirementLease; RETIREMENT_CAPACITY],
}
#[cfg(test)]
impl Retirement {
    fn testing() -> Self {
        let domain =
            crate::retirement::RetirementDomain::try_start(RETIREMENT_CAPACITY, 0).unwrap();
        Self {
            panes: std::array::from_fn(|_| domain.try_reserve().unwrap()),
        }
    }
}
struct Request {
    ready: Arc<ReadyRoot>,
    scan: FolderSizeScanId,
    selected: Option<Arc<HashSet<EntryId>>>,
    single: bool,
}
struct Observation {
    ready: Weak<ReadyRoot>,
    request: SelectionSizeId,
    revision: SelectionRevision,
    selected: Arc<HashSet<EntryId>>,
}
#[derive(Default)]
struct Pending {
    queue: VecDeque<Request>,
    observer: Option<Observation>,
    active: Option<(FolderSizeScanId, Cancellation)>,
    last_scan: Option<FolderSizeScanId>,
    last_observer: SelectionSizeId,
    in_flight: Option<(u64, u64)>,
    flush: bool,
    closed: bool,
}
struct Lane {
    pending: WorkerOwned<Mutex<Pending>>,
    wake: Condvar,
    stop: Cancellation,
}
#[cfg(test)]
impl Default for Lane {
    fn default() -> Self {
        let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
        Self::new(domain.try_reserve().unwrap())
    }
}
impl Lane {
    fn new(lease: RetirementLease) -> Self {
        Self {
            pending: lease.own(Mutex::new(Pending::default())),
            wake: Condvar::new(),
            stop: Cancellation::default(),
        }
    }
    fn close(&self) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.closed = true;
        self.stop.cancel();
        if let Some((_, cancel)) = &pending.active {
            cancel.cancel();
        }
        self.wake.notify_all();
    }
}

pub(crate) struct Controller {
    lanes: [Arc<Lane>; 2],
    workers: [Option<JoinHandle<()>>; 2],
    snapshots: Snapshots,
    outbox: Outbox,
}
impl Controller {
    pub(crate) fn new(retirement: Retirement, snapshots: Snapshots, outbox: Outbox) -> Self {
        let lanes = retirement.panes.map(|lease| Arc::new(Lane::new(lease)));
        let workers = std::array::from_fn(|index| {
            let lane = lanes[index].clone();
            let outbox = outbox.clone();
            let snapshots = snapshots.clone();
            Some(tokio::task::spawn_blocking(move || {
                worker::Worker::new(lane, outbox, snapshots).run()
            }))
        });
        Self {
            lanes,
            workers,
            snapshots,
            outbox,
        }
    }
    pub(crate) async fn submit(&self, command: Command) {
        let pane = match &command {
            Command::Compute { root, .. }
            | Command::Entry { root, .. }
            | Command::Observe { root, .. } => root.pane,
            Command::Cancel { pane, .. } | Command::Acknowledge { pane, .. } => *pane,
        };
        let lane = self.lanes[pane.index()].clone();
        let snapshots = self.snapshots.clone();
        let outbox = self.outbox.clone();
        // Exact capture is a short lock, but rejected large payloads also retire here.
        let _ = tokio::task::spawn_blocking(move || admit(lane, snapshots, outbox, command)).await;
    }
    pub(crate) async fn shutdown(mut self) {
        for lane in &self.lanes {
            lane.close();
        }
        for worker in &mut self.workers {
            if let Some(worker) = worker.take() {
                let _ = tokio::time::timeout(std::time::Duration::from_millis(200), worker).await;
            }
        }
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        for lane in &self.lanes {
            lane.close();
        }
    }
}

fn publish(outbox: &Outbox, event: Event, cancel: &Cancellation) -> bool {
    outbox.blocking_send(midnight_core::Event::ArchiveSizes(Box::new(event)), cancel)
}
fn reject(
    outbox: &Outbox,
    pane: PaneId,
    scan: Option<FolderSizeScanId>,
    message: &str,
    stop: &Cancellation,
) {
    publish(
        outbox,
        Event::Rejected {
            pane,
            scan,
            message: message.into(),
        },
        stop,
    );
}
fn admit(lane: Arc<Lane>, snapshots: Snapshots, outbox: Outbox, command: Command) {
    let (root, scan) = match &command {
        Command::Acknowledge { mount, version, .. } => {
            let mut pending = lane.pending.lock().unwrap_or_else(|e| e.into_inner());
            if pending.in_flight == Some((*mount, *version)) {
                pending.in_flight = None;
                pending.flush = true;
                lane.wake.notify_one();
            }
            return;
        }
        Command::Cancel { pane, scan } => {
            let mut pending = lane.pending.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((active, cancel)) = &pending.active
                && active == scan
            {
                cancel.cancel();
            }
            let removed = pending
                .queue
                .iter()
                .position(|request| request.scan == *scan)
                .and_then(|index| pending.queue.remove(index));
            drop(pending);
            if removed.is_some() {
                publish(
                    &outbox,
                    Event::Finished {
                        pane: *pane,
                        scan: *scan,
                        cancelled: true,
                    },
                    &lane.stop,
                );
            }
            return;
        }
        Command::Compute { root, scan, .. } | Command::Entry { root, scan, .. } => {
            (root, Some(*scan))
        }
        Command::Observe { root, .. } => (root, None),
    };
    let pane = root.pane;
    let ready = snapshots.lock().unwrap_or_else(|e| e.into_inner())[pane.index()]
        .capture_tree_root(root)
        .ok();
    let oversized = match &command {
        Command::Compute {
            selection: Some(selected),
            ..
        }
        | Command::Observe { selected, .. } => selected.capacity() > scan::MAX_NODES * 2,
        _ => false,
    };
    let Some(ready) = ready.filter(|ready| {
        ready.archive().is_some() && ready.snapshot().entries.len() <= scan::MAX_NODES && !oversized
    }) else {
        reject(
            &outbox,
            pane,
            scan,
            "Archive size request has no current captured flat listing.",
            &lane.stop,
        );
        return;
    };
    let mut pending = lane.pending.lock().unwrap_or_else(|e| e.into_inner());
    if pending.closed {
        return;
    }
    let mut retired = VecDeque::new();
    match command {
        Command::Observe {
            request,
            revision,
            selected,
            ..
        } => {
            if request <= pending.last_observer {
                return;
            }
            pending.last_observer = request;
            let old = pending.observer.replace(Observation {
                ready: Arc::downgrade(&ready),
                request,
                revision,
                selected,
            });
            lane.wake.notify_one();
            drop(pending);
            drop(old);
            return;
        }
        command @ (Command::Compute { .. } | Command::Entry { .. }) => {
            let (scan, selected, single) = match command {
                Command::Compute {
                    scan, selection, ..
                } => (scan, selection, false),
                Command::Entry { scan, entry, .. } => {
                    (scan, Some(Arc::new(HashSet::from([entry]))), true)
                }
                _ => unreachable!(),
            };
            if pending.active.as_ref().is_some_and(|(id, _)| *id == scan)
                || pending.queue.iter().any(|r| r.scan == scan)
            {
                return;
            }
            if scan.0 == 0 || pending.last_scan.is_some_and(|old| scan <= old) {
                drop(pending);
                reject(
                    &outbox,
                    pane,
                    Some(scan),
                    "Archive size request is stale.",
                    &lane.stop,
                );
                return;
            }
            if single && pending.queue.len() >= MAX_PENDING {
                drop(pending);
                reject(
                    &outbox,
                    pane,
                    Some(scan),
                    "Archive size queue is full.",
                    &lane.stop,
                );
                return;
            }
            if !single {
                if let Some((_, cancel)) = &pending.active {
                    cancel.cancel();
                }
                std::mem::swap(&mut retired, &mut pending.queue);
            }
            pending.last_scan = Some(scan);
            pending.queue.push_back(Request {
                ready,
                scan,
                selected,
                single,
            });
        }
        Command::Cancel { .. } | Command::Acknowledge { .. } => unreachable!(),
    }
    lane.wake.notify_one();
    drop(pending);
    for request in retired {
        publish(
            &outbox,
            Event::Finished {
                pane,
                scan: request.scan,
                cancelled: true,
            },
            &lane.stop,
        );
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod lifetime_tests;
