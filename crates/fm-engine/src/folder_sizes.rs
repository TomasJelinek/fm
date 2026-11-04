//! Cooperative directory-size scanning on a blocking backend worker.

use fm_core::{
    Cancellation, DirectoryService, Entry, EntryId, EntryKind, EntryMetadata, FileSystem,
    FolderSize, FsError,
};
use std::{
    mem::size_of,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
const PENDING_PATH_BUDGET: usize = 16 * 1024 * 1024;

mod batched;
pub(crate) use batched::{IoBudget, compute_batched};

/// Local adapter contract permits a borrowed, scoped scheduling wrapper without
/// adding a 'static requirement to its filesystem reference.
trait ScanSource {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError>;
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError>;
}

impl<T: DirectoryService + ?Sized> ScanSource for T {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        DirectoryService::read_directory(self, path, cancel, emit)
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        DirectoryService::entry_metadata(self, path)
    }
}

#[cfg(test)]
pub(crate) fn compute(
    filesystem: &dyn FileSystem,
    roots: &[Entry],
    cancel: &Cancellation,
    emit: &mut dyn FnMut(EntryId, FolderSize) -> bool,
) {
    compute_with_budget(filesystem, roots, cancel, emit, PENDING_PATH_BUDGET);
}

#[cfg(test)]
fn compute_with_budget(
    filesystem: &dyn FileSystem,
    roots: &[Entry],
    cancel: &Cancellation,
    emit: &mut dyn FnMut(EntryId, FolderSize) -> bool,
    pending_path_budget: usize,
) {
    for root in roots {
        if cancel.is_cancelled() {
            return;
        }
        if root.kind == EntryKind::Directory
            && !scan_root(filesystem, root, cancel, emit, pending_path_budget)
        {
            return;
        }
    }
}

/// This is a live estimate: another process may replace paths while we scan.
/// Explicit links are excluded, but the path-based provider contract cannot
/// guarantee opened-directory identity across calls. No file contents are opened.
fn scan_root<F: ScanSource + ?Sized>(
    filesystem: &F,
    root: &Entry,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(EntryId, FolderSize) -> bool,
    pending_path_budget: usize,
) -> bool {
    let mut size = FolderSize::default();
    if !emit(root.id, size) {
        return false;
    }
    let mut last_progress = Instant::now();
    let mut pending = vec![root.path.clone()];
    let mut pending_bytes = path_cost(&root.path);
    let mut receiver_open = true;

    while let Some(directory) = pending.pop() {
        pending_bytes = pending_bytes.saturating_sub(path_cost(&directory));
        if cancel.is_cancelled() {
            return false;
        }
        let result = filesystem.read_directory(&directory, cancel, &mut |entry| {
            if cancel.is_cancelled() || !receiver_open {
                return false;
            }
            if !is_immediate_child(&directory, &entry.path) {
                size.errors = size.errors.saturating_add(1);
            } else {
                match entry.kind {
                    EntryKind::Directory => {
                        let cost = path_cost(&entry.path);
                        // Bound a wide tree's frontier without imposing any limit
                        // on a single native path. An over-budget subtree is
                        // visibly partial, never silently reported as zero.
                        if pending.is_empty()
                            || pending_bytes.saturating_add(cost) <= pending_path_budget
                        {
                            pending_bytes = pending_bytes.saturating_add(cost);
                            pending.push(entry.path);
                        } else {
                            size.errors = size.errors.saturating_add(1);
                        }
                    }
                    EntryKind::File => match filesystem.entry_metadata(&entry.path) {
                        Ok(metadata) => match metadata.size {
                            Some(bytes) => {
                                add_checked(&mut size.bytes, bytes, &mut size.errors);
                                add_checked(&mut size.files, 1, &mut size.errors);
                            }
                            None => size.errors = size.errors.saturating_add(1),
                        },
                        Err(_) => size.errors = size.errors.saturating_add(1),
                    },
                    EntryKind::Symlink => {
                        add_checked(&mut size.skipped_links, 1, &mut size.errors);
                    }
                    EntryKind::Other => {}
                }
            }
            if cancel.is_cancelled() {
                return false;
            }
            if last_progress.elapsed() >= PROGRESS_INTERVAL {
                receiver_open = emit(root.id, size);
                last_progress = Instant::now();
            }
            receiver_open
        });
        if cancel.is_cancelled() || !receiver_open {
            return false;
        }
        if result.is_err() {
            size.errors = size.errors.saturating_add(1);
        }
        if last_progress.elapsed() >= PROGRESS_INTERVAL {
            if !emit(root.id, size) {
                return false;
            }
            last_progress = Instant::now();
        }
    }
    size.finished = true;
    !cancel.is_cancelled() && emit(root.id, size)
}

fn path_cost(path: &Path) -> usize {
    path.as_os_str().len().saturating_add(size_of::<PathBuf>())
}

fn is_immediate_child(directory: &Path, child: &Path) -> bool {
    let Ok(relative) = child.strip_prefix(directory) else {
        return false;
    };
    let mut components = relative.components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn add_checked(counter: &mut u64, amount: u64, errors: &mut u64) {
    match counter.checked_add(amount) {
        Some(value) => *counter = value,
        None => {
            *counter = u64::MAX;
            *errors = errors.saturating_add(1);
        }
    }
}

#[cfg(test)]
#[path = "folder_sizes/tests.rs"]
mod tests;
