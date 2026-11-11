//! Two coalescing display-only probe lanes, independent of tree adoption/selection.
use crate::{
    Outbox, Snapshots,
    providers::{NativeProvider, ProviderError},
};
use midnight_core::*;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
use tokio::{sync::Semaphore, task::JoinSet};

#[derive(Default)]
struct Pending {
    latest: Option<Arc<FolderPresenceRequest>>,
    active: Option<Cancellation>,
    retired: Vec<Arc<FolderPresenceRequest>>,
}
type Mailbox = Arc<Mutex<Pending>>;
pub(crate) struct Controller {
    lanes: [(Mailbox, mpsc::SyncSender<()>); 2],
    stop: Cancellation,
}

impl Controller {
    pub(crate) fn new(
        filesystem: Arc<dyn FileSystem>,
        snapshots: Snapshots,
        listings: [Arc<Semaphore>; 2],
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let stop = Cancellation::default();
        let lanes = std::array::from_fn(|side| {
            let lane: Mailbox = Arc::default();
            let (wake, receiver) = mpsc::sync_channel(1);
            let pending = lane.clone();
            let stopping = stop.clone();
            let snapshots = snapshots.clone();
            let filesystem = filesystem.clone();
            let listing = listings[side].clone();
            let outbox = outbox.clone();
            tasks.spawn_blocking(move || {
                worker(
                    pending, receiver, stopping, snapshots, filesystem, listing, outbox,
                )
            });
            (lane, wake)
        });
        Self { lanes, stop }
    }
    pub(crate) async fn submit(&self, request: Arc<FolderPresenceRequest>) {
        let (lane, wake) = &self.lanes[request.root.pane.index()];
        let retired = {
            let mut pending = lane.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(active) = pending.active.take() {
                active.cancel();
            }
            if let Some(old) = pending.latest.replace(request) {
                old.cancel.cancel();
                pending.retired.push(old);
            }
            (pending.retired.len() >= 8).then(|| std::mem::take(&mut pending.retired))
        };
        if let Some(retired) = retired {
            let _ = tokio::task::spawn_blocking(move || drop(retired)).await;
        }
        let _ = wake.try_send(());
    }
    pub(crate) fn cancel(&self, pane: PaneId) {
        let mut lane = self.lanes[pane.index()]
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(active) = lane.active.take() {
            active.cancel();
        }
        if let Some(request) = &lane.latest {
            request.cancel.cancel();
        }
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.stop.cancel();
        for pane in [PaneId::Left, PaneId::Right] {
            self.cancel(pane);
        }
    }
}

fn worker(
    lane: Mailbox,
    wake: mpsc::Receiver<()>,
    stop: Cancellation,
    snapshots: Snapshots,
    filesystem: Arc<dyn FileSystem>,
    listing: Arc<Semaphore>,
    outbox: Outbox,
) {
    let native = NativeProvider::new(filesystem);
    loop {
        let (request, retired) = {
            let mut lane = lane.lock().unwrap_or_else(|e| e.into_inner());
            let request = lane.latest.take();
            if let Some(request) = &request {
                lane.active = Some(request.cancel.clone());
            }
            (request, std::mem::take(&mut lane.retired))
        };
        drop(retired);
        if stop.is_cancelled() {
            break;
        }
        let Some(request) = request else {
            if matches!(
                wake.recv_timeout(Duration::from_millis(25)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ) {
                break;
            }
            continue;
        };
        let cancel = request.cancel.linked(&stop);
        let result = run(&request, &cancel, &snapshots, &native, &listing);
        if cancel.is_cancelled() {
            continue;
        }
        if let Some(values) = result {
            let event = Event::FolderPresence(Box::new(FolderPresenceSnapshot {
                root: request.root.clone(),
                revision: request.revision,
                values: values.into(),
            }));
            outbox.blocking_send(event, &cancel);
        }
    }
    let retired = {
        let mut lane = lane.lock().unwrap_or_else(|e| e.into_inner());
        (lane.latest.take(), std::mem::take(&mut lane.retired))
    };
    drop(retired);
}

fn run(
    request: &FolderPresenceRequest,
    cancel: &Cancellation,
    snapshots: &Snapshots,
    native: &NativeProvider,
    listing: &Arc<Semaphore>,
) -> Option<Vec<(EntryId, FolderPresence)>> {
    if request.revision.0 == 0
        || request.visible.len() > FolderPresenceRequest::MAX_VISIBLE
        || cancel.is_cancelled()
    {
        return None;
    }
    let ready = snapshots.lock().unwrap_or_else(|e| e.into_inner())[request.root.pane.index()]
        .capture_tree_root(&request.root)
        .ok()?;
    let mut seen = HashSet::new();
    for id in &request.visible {
        if cancel.is_cancelled() {
            return None;
        }
        let entry = request
            .root
            .index
            .by_id
            .get(id)
            .and_then(|position| request.root.entries.get(*position))?;
        if entry.id != *id || entry.kind != EntryKind::Directory || !seen.insert(*id) {
            return None;
        }
    }
    let mut values = Vec::with_capacity(request.visible.len());
    let report = crate::tree_diagnostics::log();
    for id in &request.visible {
        let entry = &request.root.entries[request.root.index.by_id[id]];
        let _listing = loop {
            if cancel.is_cancelled() {
                return None;
            }
            match listing.clone().try_acquire_owned() {
                Ok(permit) => break permit,
                Err(tokio::sync::TryAcquireError::NoPermits) => {
                    std::thread::park_timeout(Duration::from_millis(10))
                }
                Err(tokio::sync::TryAcquireError::Closed) => return None,
            }
        };
        let gate = midnight_work_budget::shared_io();
        let _io = gate.acquire(midnight_work_budget::IO_SLOTS, || cancel.is_cancelled())?;
        let value = (|| {
            if let Some(location) = ready.archive() {
                let node = entry
                    .virtual_node
                    .as_ref()
                    .ok_or(ProviderError::StaleReference)?;
                let mount = location.mount.as_ref().ok_or(ProviderError::Unavailable)?;
                if node.parent != location.directory
                    || node.id.mount != mount.id()
                    || node.kind != EntryKind::Directory
                    || location
                        .trail
                        .iter()
                        .any(|(ancestor, _)| *ancestor == node.id.entry)
                {
                    return Err(ProviderError::StaleReference);
                }
                let mut child = location.clone();
                child.directory = node.id.entry;
                child.address.components.push(node.raw_name.to_vec());
                child.trail.push((node.id.entry, node.display_name.clone()));
                let opened = crate::providers::captured_archive(&child, cancel)?;
                crate::providers::folder_probe::probe(&opened, cancel)
            } else {
                let path = entry.native_path().ok_or(ProviderError::StaleReference)?;
                native.has_subfolders(path, cancel)
            }
        })();
        values.push((
            *id,
            match value {
                Ok(true) => FolderPresence::HasFolders,
                Ok(false) => FolderPresence::Empty,
                Err(error) => {
                    let message = error.to_string();
                    let details = crate::tree_diagnostics::report(
                        &report,
                        format_args!("{} ({:?})", entry.display_name, entry.path),
                        &message,
                    );
                    FolderPresence::Unavailable(TreeDiagnostic::with_details(&message, details))
                }
            },
        ));
    }
    if cancel.is_cancelled()
        || snapshots.lock().unwrap_or_else(|e| e.into_inner())[request.root.pane.index()]
            .capture_tree_root(&request.root)
            .is_err()
    {
        return None;
    }
    Some(values)
}

#[cfg(test)]
#[path = "folder_presence_tests.rs"]
mod tests;
