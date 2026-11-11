//! Cooperative directory-size scanning on a blocking backend worker.

use midnight_core::{
    Cancellation, DirectoryService, Entry, EntryId, EntryKind, EntryMetadata, FileSystem,
    FolderSize, FsError, SizeDirectoryIdentity, SizeDirectoryTarget,
};
use std::{
    collections::HashSet,
    io,
    mem::size_of,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
const PENDING_PATH_BUDGET: usize = 16 * 1024 * 1024;

mod batched;
mod warnings;
use crate::size_warnings::ReportLog;
pub(crate) use batched::{IoBudget, compute_batched};
use warnings::ScanReport;

/// Local adapter contract permits a borrowed, scoped scheduling wrapper without
/// adding a 'static requirement to its filesystem reference.
trait ScanSource {
    fn resolve_size_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<SizeDirectoryTarget>, FsError>;
    fn finish_size_resolution(&self) {}
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError>;
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError>;
}

impl<T: DirectoryService + ?Sized> ScanSource for T {
    fn resolve_size_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<SizeDirectoryTarget>, FsError> {
        DirectoryService::resolve_size_directory(self, path, cancel)
    }
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
    let report = ReportLog::for_scan();
    for root in roots {
        if cancel.is_cancelled() {
            return;
        }
        if matches!(root.kind, EntryKind::Directory | EntryKind::DirectoryLink)
            && !scan_root(filesystem, root, cancel, emit, pending_path_budget, &report)
        {
            return;
        }
    }
}

/// This is a live estimate: another process may replace paths while we scan.
/// Native directory links are resolved for sizing; active ancestry identities
/// stop cycles. Noncyclic aliases count their logical contents independently.
/// Paths can change between identity and listing calls; this is not a snapshot.
/// File links and special-file contents are not opened.
fn scan_root<F: ScanSource + ?Sized>(
    filesystem: &F,
    root: &Entry,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(EntryId, FolderSize) -> bool,
    pending_path_budget: usize,
    report: &ReportLog,
) -> bool {
    if cancel.is_cancelled() {
        return false;
    }
    let report = ScanReport::new(report);
    let mut size = FolderSize::default();
    if !emit(root.id, size.clone()) {
        return false;
    }
    let mut last_progress = Instant::now();
    let mut pending = vec![Pending::Enter(root.path.clone(), root.kind)];
    let mut ancestors = HashSet::new();
    let mut pending_bytes = path_cost(&root.path);
    let mut receiver_open = true;

    while let Some(next) = pending.pop() {
        let (path, kind) = match next {
            Pending::Leave(identity) => {
                ancestors.remove(&identity);
                continue;
            }
            Pending::Enter(path, kind) => (path, kind),
        };
        pending_bytes = pending_bytes.saturating_sub(path_cost(&path));
        if cancel.is_cancelled() {
            report.finish(&mut size, true);
            emit(root.id, size);
            return false;
        }
        // The scheduled adapter may retain its resolution I/O ticket through
        // enumeration. Release it on every cycle/error/cancellation path too.
        let _resolution = ResolutionPermit(filesystem);
        let directory = match filesystem.resolve_size_directory(&path, cancel) {
            Ok(Some(target)) => {
                if !ancestors.insert(target.identity) {
                    // An intentional cycle omission is not an inaccessible-target error.
                    count_skipped(&report, &mut size, &path);
                    continue;
                }
                pending.push(Pending::Leave(target.identity));
                target.path
            }
            Ok(None) if matches!(kind, EntryKind::Symlink | EntryKind::DirectoryLink) => {
                count_skipped(&report, &mut size, &path);
                continue;
            }
            Ok(None) => {
                report.issue(
                    &mut size,
                    &path,
                    "resolve directory size target",
                    io::ErrorKind::NotADirectory,
                    "The captured directory is no longer a directory",
                );
                continue;
            }
            Err(error)
                if error.kind == io::ErrorKind::Unsupported && kind == EntryKind::Directory =>
            {
                path
            }
            Err(error) => {
                if !cancel.is_cancelled()
                    && matches!(kind, EntryKind::Symlink | EntryKind::DirectoryLink)
                {
                    count_skipped(&report, &mut size, &path);
                }
                report.provider_error(&mut size, &error, cancel);
                continue;
            }
        };
        let result = filesystem.read_directory(&directory, cancel, &mut |entry| {
            if cancel.is_cancelled() || !receiver_open {
                return false;
            }
            if !is_immediate_child(&directory, &entry.path) {
                report.issue(&mut size, &entry.path, "validate directory entry", io::ErrorKind::InvalidData, "Provider returned a path outside the immediate directory");
            } else {
                match entry.kind {
                    EntryKind::Directory | EntryKind::DirectoryLink | EntryKind::Symlink => {
                        let cost = path_cost(&entry.path);
                        // Bound a wide tree's frontier without imposing any limit
                        // on a single native path. An over-budget subtree is
                        // visibly partial, never silently reported as zero.
                        if pending_bytes == 0
                            || pending_bytes.saturating_add(cost) <= pending_path_budget
                        {
                            pending_bytes = pending_bytes.saturating_add(cost);
                            pending.push(Pending::Enter(entry.path, entry.kind));
                        } else {
                            report.issue(&mut size, &entry.path, "queue directory", io::ErrorKind::OutOfMemory, "Directory frontier budget exhausted; this subtree was not counted");
                        }
                    }
                    EntryKind::File => match filesystem.entry_metadata(&entry.path) {
                        Ok(metadata) => match metadata.size {
                            Some(bytes) => {
                                if !add_checked(&mut size.bytes, bytes) {
                                    report.issue(&mut size, &entry.path, "sum file lengths", io::ErrorKind::InvalidData, "Total byte count exceeds the supported integer range");
                                }
                                if !add_checked(&mut size.files, 1) {
                                    report.issue(&mut size, &entry.path, "count files", io::ErrorKind::InvalidData, "File count exceeds the supported integer range");
                                }
                            }
                            None => report.issue(&mut size, &entry.path, "read file length", io::ErrorKind::InvalidData, "Provider metadata did not contain a file length"),
                        },
                        Err(error) => report.provider_error(&mut size, &error, cancel),
                    },
                    EntryKind::Other => {}
                }
            }
            if cancel.is_cancelled() {
                return false;
            }
            if last_progress.elapsed() >= PROGRESS_INTERVAL {
                report.publish(&mut size);
                receiver_open = emit(root.id, size.clone());
                last_progress = Instant::now();
            }
            receiver_open
        });
        if let Err(error) = result {
            report.provider_error(&mut size, &error, cancel);
        }
        if !receiver_open {
            return false;
        }
        if cancel.is_cancelled() {
            report.finish(&mut size, true);
            emit(root.id, size);
            return false;
        }
        if last_progress.elapsed() >= PROGRESS_INTERVAL {
            report.publish(&mut size);
            if !emit(root.id, size.clone()) {
                return false;
            }
            last_progress = Instant::now();
        }
    }
    let cancelled = cancel.is_cancelled();
    report.finish(&mut size, cancelled);
    let accepted = emit(root.id, size);
    !cancelled && accepted
}

enum Pending {
    Enter(PathBuf, EntryKind),
    Leave(SizeDirectoryIdentity),
}

struct ResolutionPermit<'a, F: ScanSource + ?Sized>(&'a F);
impl<F: ScanSource + ?Sized> Drop for ResolutionPermit<'_, F> {
    fn drop(&mut self) {
        self.0.finish_size_resolution();
    }
}

fn count_skipped(report: &ScanReport<'_>, size: &mut FolderSize, path: &Path) {
    if !add_checked(&mut size.skipped_links, 1) {
        report.issue(
            size,
            path,
            "count skipped links or directory cycles",
            io::ErrorKind::InvalidData,
            "Skipped link/cycle count exceeds the supported integer range",
        );
    }
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

fn add_checked(counter: &mut u64, amount: u64) -> bool {
    match counter.checked_add(amount) {
        Some(value) => {
            *counter = value;
            true
        }
        None => {
            *counter = u64::MAX;
            false
        }
    }
}

#[cfg(test)]
#[path = "folder_sizes/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "folder_sizes/warning_tests.rs"]
mod warning_tests;

#[cfg(test)]
#[path = "folder_sizes/native_tests.rs"]
mod native_tests;
