//! Fair storage-aware scheduling for a bounded batch of visible root folders.

use super::*;
use midnight_core::StorageKind;
use midnight_work_budget::{Admission, Gate, MAX_IO_LIMIT, Permit};
use std::{
    cell::{Cell, RefCell},
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

const QUANTUM_ENTRIES: usize = 256;
const QUANTUM_TIME: Duration = Duration::from_millis(100);
const WAIT_INTERVAL: Duration = Duration::from_millis(10);

/// Shared across both pane operations. Classification is a separate, single
/// control query; unknown/network/rotational bulk I/O is exclusive.
pub(crate) struct IoBudget {
    io: Arc<Gate>,
    classification: Arc<Gate>,
}

impl IoBudget {
    /// A private gate for tests that must not contend with the process-wide one.
    #[cfg(test)]
    pub(super) fn isolated() -> Self {
        Self {
            io: Arc::new(Gate::new(midnight_work_budget::DEFAULT_IO_LIMIT)),
            classification: Arc::new(Gate::new(1)),
        }
    }
    pub(crate) fn new() -> Self {
        Self {
            io: midnight_work_budget::shared_io(),
            classification: Arc::new(Gate::new(1)),
        }
    }
    fn acquire_io(self: &Arc<Self>, admission: Admission, cancel: &Cancellation) -> Option<Permit> {
        self.io.acquire(admission, || cancel.is_cancelled())
    }
    fn acquire_classification(self: &Arc<Self>, cancel: &Cancellation) -> Option<Permit> {
        self.classification
            .acquire(Admission::Exclusive, || cancel.is_cancelled())
    }
}

impl Default for IoBudget {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn compute_batched(
    filesystem: &dyn FileSystem,
    roots: &[Entry],
    cancel: &Cancellation,
    emit: &mut dyn FnMut(EntryId, FolderSize) -> bool,
    budget: &Arc<IoBudget>,
    reports: &crate::ReportArena,
) {
    if cancel.is_cancelled() {
        return;
    }
    let next = AtomicUsize::new(0);
    let local_cancel = cancel.child();
    let (progress, receiver) = mpsc::sync_channel(8);
    // Existing teams keep their scoped lanes; the shared gate applies changes to every I/O.
    // Four admitted teams can have 128 root lanes at Manual 32. Frontiers stay lazily
    // allocated per scan; this does not allocate 128 eager 16 MiB buffers.
    let workers = usize::from(budget.io.effective_limit().min(MAX_IO_LIMIT)).min(roots.len());
    thread::scope(|scope| {
        for _ in 0..workers {
            let progress = progress.clone();
            let next = &next;
            let local_cancel = &local_cancel;
            scope.spawn(move || {
                // Sequential roots on a lane share one lazily created private report.
                let report = ReportLog::for_scan(reports);
                let source = BudgetedSource {
                    filesystem,
                    budget,
                    cancel: local_cancel,
                    active: Cell::new(false),
                    resolution_permit: RefCell::new(None),
                    devices: RefCell::new(Vec::new()),
                    last: Cell::new(None),
                };
                while !local_cancel.is_cancelled() {
                    let Some(root) = roots.get(next.fetch_add(1, Ordering::Relaxed)) else {
                        break;
                    };
                    if matches!(root.kind, EntryKind::Directory | EntryKind::DirectoryLink)
                        && !scan_root(
                            &source,
                            root,
                            local_cancel,
                            &mut |entry, size| send_progress(&progress, local_cancel, entry, size),
                            SCAN_LIMITS,
                            &report,
                        )
                    {
                        break;
                    }
                }
            });
        }
        drop(progress);
        loop {
            match receiver.recv_timeout(WAIT_INTERVAL) {
                Ok((entry, size)) => {
                    if !emit(entry, size) {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        local_cancel.cancel();
        drop(receiver);
        // Scoped threads retain every borrowed provider/path until their real
        // OS call returns. Cancellation cannot interrupt an uncancellable call.
    });
}

fn send_progress(
    sender: &mpsc::SyncSender<(EntryId, FolderSize)>,
    cancel: &Cancellation,
    entry: EntryId,
    size: FolderSize,
) -> bool {
    let finished = size.finished || cancel.is_cancelled();
    let mut event = (entry, size);
    loop {
        match sender.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
            // Optional running totals must never hold up a native iterator.
            Err(mpsc::TrySendError::Full(_)) if !finished => return true,
            Err(mpsc::TrySendError::Full(unsent)) => event = unsent,
        }
        // Final totals are sent after the directory permit has been released.
        thread::sleep(WAIT_INTERVAL);
    }
}

struct BudgetedSource<'a> {
    filesystem: &'a dyn FileSystem,
    budget: &'a Arc<IoBudget>,
    cancel: &'a Cancellation,
    active: Cell<bool>,
    /// Admission verified for the next enumeration, with the resolution ticket
    /// when that ticket already has the same admission.
    resolution_permit: RefCell<Option<(Admission, Option<Permit>)>>,
    /// Storage classification is an expensive serialized control query. A lane
    /// classifies each resolved native device once; providers without
    /// identities classify every enumerated directory.
    devices: RefCell<Vec<(u64, Admission)>>,
    /// Admission of the last resolved directory, used for the next identity query.
    last: Cell<Option<Admission>>,
}

/// A lane rarely crosses more devices; older ones are classified again.
const KNOWN_DEVICES: usize = 16;

impl BudgetedSource<'_> {
    fn io_admission(&self, path: &Path, cancel: &Cancellation) -> Option<Admission> {
        let _classification = self.budget.acquire_classification(cancel)?;
        let kind = self
            .filesystem
            .storage_kind(path)
            .unwrap_or(StorageKind::Unknown);
        Some(midnight_work_budget::storage_admission(
            kind == StorageKind::LocalSsd,
        ))
    }

    fn known_device(&self, device: u64) -> Option<Admission> {
        self.devices
            .borrow()
            .iter()
            .find_map(|&(known, admission)| (known == device).then_some(admission))
    }

    fn remember_device(&self, device: u64, admission: Admission) {
        let mut devices = self.devices.borrow_mut();
        if devices.len() == KNOWN_DEVICES {
            devices.remove(0);
        }
        devices.push((device, admission));
    }
}

impl ScanSource for BudgetedSource<'_> {
    fn resolve_size_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<SizeDirectoryTarget>, FsError> {
        // The identity query itself uses the previous directory's admission; a
        // lane without one classifies this path first.
        let (admission, classified) = match self.last.get() {
            Some(admission) => (Some(admission), false),
            None => (self.io_admission(path, cancel), true),
        };
        let Some((admission, permit)) = admission.and_then(|admission| {
            self.budget
                .acquire_io(admission, cancel)
                .map(|permit| (admission, permit))
        }) else {
            return Err(FsError::from_io(
                "resolve directory size target",
                path,
                io::ErrorKind::Interrupted.into(),
            ));
        };
        let result = self.filesystem.resolve_size_directory(path, cancel);
        let target = match &result {
            Ok(Some(target)) => {
                let device = target.identity.device;
                let known = self.known_device(device).or_else(|| {
                    let known = if classified && target.path == path {
                        Some(admission)
                    } else {
                        // A selected root link or a mount point can change device.
                        self.io_admission(&target.path, cancel)
                    };
                    if let Some(known) = known {
                        self.remember_device(device, known);
                    }
                    known
                });
                self.last.set(known);
                known
            }
            // Without an identity every enumerated path keeps its own fresh
            // classification (for example an unannounced nested mount).
            _ => {
                self.last.set(None);
                classified.then_some(admission)
            }
        };
        // Keep the ticket through the following enumeration. Otherwise a small
        // waiting root spends its only fair turn resolving identity, then queues
        // behind the large iterator again without counting any contents.
        *self.resolution_permit.borrow_mut() =
            target.map(|target| (target, (target == admission).then_some(permit)));
        result
    }
    fn finish_size_resolution(&self) {
        self.resolution_permit.take();
    }
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        let (admission, mut permit) = match self.resolution_permit.take() {
            Some(resolved) => resolved,
            None => match self.io_admission(path, cancel) {
                Some(admission) => (admission, None),
                None => return Ok(()),
            },
        };
        if permit.is_none() {
            permit = self.budget.acquire_io(admission, cancel);
        }
        if permit.is_none() {
            return Ok(());
        }
        self.active.set(true);
        let mut entries = 0;
        let mut since_yield = Instant::now();
        let result = self.filesystem.read_directory(path, cancel, &mut |entry| {
            if cancel.is_cancelled() || !emit(entry) {
                return false;
            }
            entries += 1;
            if entries >= QUANTUM_ENTRIES || since_yield.elapsed() >= QUANTUM_TIME {
                self.active.set(false);
                drop(permit.take());
                permit = self.budget.acquire_io(admission, cancel);
                self.active.set(permit.is_some());
                entries = 0;
                since_yield = Instant::now();
            }
            permit.is_some() && !cancel.is_cancelled()
        });
        self.active.set(false);
        drop(permit);
        result
    }

    fn listing_metadata_complete(&self, entry: &Entry) -> bool {
        self.filesystem.listing_metadata_complete(entry)
    }

    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        // scan_root invokes metadata synchronously inside its enumeration
        // callback: re-acquiring here would deadlock a conservative exclusive scan.
        if !self.active.get() || self.cancel.is_cancelled() {
            return Err(FsError::from_io(
                "folder size metadata",
                path,
                io::ErrorKind::Interrupted.into(),
            ));
        }
        self.filesystem.entry_metadata(path)
    }
}

#[cfg(test)]
#[path = "batched/tests.rs"]
mod tests;
