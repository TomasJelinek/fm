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
    #[cfg(test)]
    fn isolated() -> Self {
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
                            PENDING_PATH_BUDGET,
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
    resolution_permit: RefCell<Option<(Admission, Permit)>>,
}

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
}

impl ScanSource for BudgetedSource<'_> {
    fn resolve_size_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<SizeDirectoryTarget>, FsError> {
        let Some((admission, permit)) = self.io_admission(path, cancel).and_then(|admission| {
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
        // Keep the ticket through the following enumeration. Otherwise a small
        // waiting root spends its only fair turn resolving identity, then queues
        // behind the large iterator again without counting any contents.
        *self.resolution_permit.borrow_mut() = Some((admission, permit));
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
        let Some(admission) = self.io_admission(path, cancel) else {
            return Ok(());
        };
        let mut permit = self
            .resolution_permit
            .take()
            .and_then(|(held_admission, permit)| (held_admission == admission).then_some(permit));
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
