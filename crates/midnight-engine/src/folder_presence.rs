//! Two coalescing display-only probe lanes, independent of tree adoption/selection.
use crate::{
    Outbox, Snapshots,
    providers::{NativeProvider, ProviderError},
    task_lifetime::TaskLifetime,
};
use midnight_core::*;
use midnight_work_budget::{Admission, Gate, Permit};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
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
        scope: TaskLifetime,
        reports: crate::ReportArena,
        filesystem: Arc<dyn FileSystem>,
        snapshots: Snapshots,
        listings: [Arc<Semaphore>; 2],
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let stop = Cancellation::default();
        // Both lanes serialize their storage classification queries.
        let classification = Arc::new(Gate::new(1));
        let lanes = std::array::from_fn(|side| {
            let lane: Mailbox = Arc::default();
            let (wake, receiver) = mpsc::sync_channel(1);
            let pending = lane.clone();
            let stopping = stop.clone();
            let snapshots = snapshots.clone();
            let filesystem = filesystem.clone();
            let reports = reports.clone();
            let listing = listings[side].clone();
            let outbox = outbox.clone();
            let admissions =
                Admissions::new(midnight_work_budget::shared_io(), classification.clone());
            crate::actor_thread::spawn(
                "midnight-folder-presence",
                tasks,
                scope.blocking(move || {
                    worker(
                        pending,
                        receiver,
                        stopping,
                        WorkerContext {
                            snapshots,
                            filesystem,
                            listing,
                            outbox,
                            reports,
                            admissions,
                        },
                    )
                }),
            );
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

struct WorkerContext {
    snapshots: Snapshots,
    filesystem: Arc<dyn FileSystem>,
    listing: Arc<Semaphore>,
    outbox: Outbox,
    reports: crate::ReportArena,
    admissions: Admissions,
}

/// One lane's probe admission. Storage classification is a serialized control
/// query (on Linux it reads the mount table), so a lane classifies its listed
/// directory once and each other device a row resolves to (through a link or
/// a mount point) once, until it lists another directory. Only a row proven
/// to be on a confirmed local SSD shares the I/O gate with running scans.
struct Admissions {
    io: Arc<Gate>,
    classification: Arc<Gate>,
    listing: Option<Listing>,
}

struct Listing {
    directory: PathBuf,
    admission: Admission,
    /// The listed directory's device, queried only for a shared listing.
    device: Option<u64>,
    /// Other devices that rows of this listing resolved to.
    devices: Vec<(u64, Admission)>,
}

/// A listing rarely reaches more devices; older ones are classified again.
const KNOWN_DEVICES: usize = 16;

impl Admissions {
    fn new(io: Arc<Gate>, classification: Arc<Gate>) -> Self {
        Self {
            io,
            classification,
            listing: None,
        }
    }

    fn classify(
        &self,
        filesystem: &dyn FileSystem,
        path: &Path,
        cancel: &Cancellation,
    ) -> Option<Admission> {
        let _classification = self
            .classification
            .acquire(Admission::Exclusive, || cancel.is_cancelled())?;
        let kind = filesystem
            .storage_kind(path)
            .unwrap_or(StorageKind::Unknown);
        Some(midnight_work_budget::storage_admission(
            kind == StorageKind::LocalSsd,
        ))
    }

    /// Classify `directory`, the parent of every probed row, unless the lane
    /// already did for this listing. `None` only after cancellation.
    fn list(
        &mut self,
        filesystem: &dyn FileSystem,
        directory: &Path,
        cancel: &Cancellation,
    ) -> Option<()> {
        if self
            .listing
            .as_ref()
            .is_some_and(|listing| listing.directory == directory)
        {
            return Some(());
        }
        self.listing = None;
        let admission = self.classify(filesystem, directory, cancel)?;
        // Only a shared listing needs its device, to prove where its rows are.
        let device = if admission == Admission::Shared {
            let _io = self.io.acquire(admission, || cancel.is_cancelled())?;
            device(filesystem, directory, cancel)
        } else {
            None
        };
        if cancel.is_cancelled() {
            // An interrupted identity query must not leave the listing unproven.
            return None;
        }
        self.listing = Some(Listing {
            directory: directory.to_owned(),
            admission,
            device,
            devices: Vec::new(),
        });
        Some(())
    }

    fn exclusive(&self, cancel: &Cancellation) -> Option<Permit> {
        self.io
            .acquire(Admission::Exclusive, || cancel.is_cancelled())
    }

    /// The I/O permit for probing `row`. A row shares the gate only when its
    /// resolved device is the shared listing's or another classified SSD;
    /// rows of other listings, unknown identities and other storage stay
    /// exclusive. `None` only after cancellation.
    fn admit(
        &mut self,
        filesystem: &dyn FileSystem,
        row: &Path,
        cancel: &Cancellation,
    ) -> Option<Permit> {
        let Some((admission, expected)) = self.listing.as_ref().and_then(|listing| {
            listing
                .device
                .filter(|_| row.parent() == Some(listing.directory.as_path()))
                .map(|device| (listing.admission, device))
        }) else {
            return self.exclusive(cancel);
        };
        // The identity query reads metadata only, under the listing's
        // admission; a link or a mount point can lead to another device.
        let permit = self.io.acquire(admission, || cancel.is_cancelled())?;
        let resolved = device(filesystem, row, cancel);
        if resolved == Some(expected) {
            return Some(permit);
        }
        drop(permit);
        let Some(other) = resolved else {
            return self.exclusive(cancel);
        };
        let known = self.listing.as_ref().and_then(|listing| {
            listing
                .devices
                .iter()
                .find_map(|&(device, admission)| (device == other).then_some(admission))
        });
        let admission = match known {
            Some(admission) => admission,
            None => {
                let admission = self.classify(filesystem, row, cancel)?;
                if let Some(listing) = &mut self.listing {
                    if listing.devices.len() == KNOWN_DEVICES {
                        listing.devices.remove(0);
                    }
                    listing.devices.push((other, admission));
                }
                admission
            }
        };
        self.io.acquire(admission, || cancel.is_cancelled())
    }
}

fn device(filesystem: &dyn FileSystem, path: &Path, cancel: &Cancellation) -> Option<u64> {
    filesystem
        .browse_directory_identity(path, cancel)
        .ok()
        .flatten()
        .map(|identity| identity.device)
}

fn worker(lane: Mailbox, wake: mpsc::Receiver<()>, stop: Cancellation, context: WorkerContext) {
    let WorkerContext {
        snapshots,
        filesystem,
        listing,
        outbox,
        reports,
        mut admissions,
    } = context;
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
            // Every submission wakes the lane; dropping the controller
            // disconnects it. No idle timer is needed.
            if wake.recv().is_err() {
                break;
            }
            continue;
        };
        let cancel = request.cancel.linked(&stop);
        let result = run(
            &request,
            &cancel,
            &snapshots,
            &native,
            &listing,
            &reports,
            &mut admissions,
        );
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
    reports: &crate::ReportArena,
    admissions: &mut Admissions,
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
    let native_root = ready.archive().is_none();
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
        let directory = entry.kind == EntryKind::Directory
            || (native_root && entry.kind == EntryKind::DirectoryLink);
        if entry.id != *id || !directory || !seen.insert(*id) {
            return None;
        }
    }
    let mut values = Vec::with_capacity(request.visible.len());
    let report = crate::tree_diagnostics::log(reports);
    // Proven local SSD probes share the process-wide gate with running scans
    // instead of draining them for every folder; archives, other storage and
    // failed hints stay exclusive.
    let filesystem = native.filesystem();
    if native_root
        && let Some(directory) = request
            .visible
            .first()
            .and_then(|id| request.root.entries[request.root.index.by_id[id]].native_path())
            .and_then(Path::parent)
    {
        admissions.list(filesystem.as_ref(), directory, cancel)?;
    }
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
        let _io = match entry.native_path().filter(|_| native_root) {
            Some(path) => admissions.admit(filesystem.as_ref(), path, cancel)?,
            None => admissions.exclusive(cancel)?,
        };
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
