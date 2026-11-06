//! Fair storage-aware scheduling for a bounded batch of visible root folders.

use super::*;
use midnight_core::StorageKind;
use std::{
    cell::Cell,
    collections::VecDeque,
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

const ROOT_WORKERS: usize = 4;
const IO_SLOTS: u8 = 4;
const QUANTUM_ENTRIES: usize = 256;
const QUANTUM_TIME: Duration = Duration::from_millis(100);
const WAIT_INTERVAL: Duration = Duration::from_millis(10);

/// Shared across both pane operations. Classification is a separate, single
/// control query; unknown/network/rotational bulk I/O takes all four I/O slots.
pub(crate) struct IoBudget {
    io: Arc<Gate>,
    classification: Arc<Gate>,
}

impl IoBudget {
    pub(crate) fn new() -> Self {
        Self {
            io: Arc::new(Gate::new(IO_SLOTS)),
            classification: Arc::new(Gate::new(1)),
        }
    }
    fn acquire_io(self: &Arc<Self>, cost: u8, cancel: &Cancellation) -> Option<Permit> {
        self.io.acquire(cost, cancel)
    }
    fn acquire_classification(self: &Arc<Self>, cancel: &Cancellation) -> Option<Permit> {
        self.classification.acquire(1, cancel)
    }
}

impl Default for IoBudget {
    fn default() -> Self {
        Self::new()
    }
}

struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

struct GateState {
    available: u8,
    next: u64,
    queue: VecDeque<(u64, u8)>,
}

impl Gate {
    fn new(capacity: u8) -> Self {
        Self {
            state: Mutex::new(GateState {
                available: capacity,
                next: 0,
                queue: VecDeque::new(),
            }),
            changed: Condvar::new(),
        }
    }

    fn acquire(self: &Arc<Self>, cost: u8, cancel: &Cancellation) -> Option<Permit> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let ticket = state.next;
        state.next = state.next.wrapping_add(1);
        state.queue.push_back((ticket, cost));
        loop {
            if cancel.is_cancelled() {
                if let Some(index) = state.queue.iter().position(|(id, _)| *id == ticket) {
                    state.queue.remove(index);
                }
                self.changed.notify_all();
                return None;
            }
            if state.queue.front().is_some_and(|(id, _)| *id == ticket) && state.available >= cost {
                state.queue.pop_front();
                state.available -= cost;
                self.changed.notify_all();
                return Some(Permit {
                    gate: self.clone(),
                    cost,
                });
            }
            state = self
                .changed
                .wait_timeout(state, WAIT_INTERVAL)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
}

struct Permit {
    gate: Arc<Gate>,
    cost: u8,
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = self
            .gate
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.available += self.cost;
        self.gate.changed.notify_all();
    }
}

pub(crate) fn compute_batched(
    filesystem: &dyn FileSystem,
    roots: &[Entry],
    cancel: &Cancellation,
    emit: &mut dyn FnMut(EntryId, FolderSize) -> bool,
    budget: &Arc<IoBudget>,
) {
    if cancel.is_cancelled() {
        return;
    }
    let next = AtomicUsize::new(0);
    let local_cancel = cancel.child();
    let (progress, receiver) = mpsc::sync_channel(8);
    thread::scope(|scope| {
        for _ in 0..ROOT_WORKERS.min(roots.len()) {
            let progress = progress.clone();
            let next = &next;
            let local_cancel = &local_cancel;
            scope.spawn(move || {
                let source = BudgetedSource {
                    filesystem,
                    budget,
                    cancel: local_cancel,
                    active: Cell::new(false),
                };
                while !local_cancel.is_cancelled() {
                    let Some(root) = roots.get(next.fetch_add(1, Ordering::Relaxed)) else {
                        break;
                    };
                    if root.kind == EntryKind::Directory
                        && !scan_root(
                            &source,
                            root,
                            local_cancel,
                            &mut |entry, size| send_progress(&progress, local_cancel, entry, size),
                            PENDING_PATH_BUDGET,
                        )
                    {
                        break;
                    }
                }
            });
        }
        drop(progress);
        while !cancel.is_cancelled() {
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
    let mut event = (entry, size);
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        match sender.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
            // Optional running totals must never hold up a native iterator.
            Err(mpsc::TrySendError::Full(_)) if !size.finished => return true,
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
}

impl ScanSource for BudgetedSource<'_> {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        let kind = {
            let Some(_classification) = self.budget.acquire_classification(cancel) else {
                return Ok(());
            };
            self.filesystem
                .storage_kind(path)
                .unwrap_or(StorageKind::Unknown)
        };
        let cost = if kind == StorageKind::LocalSsd {
            1
        } else {
            IO_SLOTS
        };
        let mut permit = self.budget.acquire_io(cost, cancel);
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
                permit = self.budget.acquire_io(cost, cancel);
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
        // callback: re-acquiring here would deadlock a conservative 4-slot scan.
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
