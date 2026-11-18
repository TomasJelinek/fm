//! Candidate private spool roots and the removal of roots that serve one process.
//! Platform-neutral policy shared by every backend that opens spools in private
//! roots, such as archive staging, diagnostic reports and file lists.
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError, Weak},
};

/// One candidate private spool root, as a backend lists them in preference order.
/// `PrivateSpoolDirectory::open` refuses a root that is not an owner-only directory
/// of this user, so the backend tries its next candidate instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpoolRoot {
    pub path: PathBuf,
    /// Only this process uses the root, so it is removed after its last spool closes
    /// instead of accumulating one directory per run.
    pub per_process: bool,
}

impl SpoolRoot {
    /// The guard that keeps a per-process root while this process may use it, or
    /// `None` for a root kept for later runs. Take it before opening the root and drop
    /// it after every spool and directory handle in the root: the last guard of this
    /// process removes the root once it is empty. Every holder of the same root shares
    /// one guard, so no holder sees its root disappear.
    pub fn guard(&self) -> Option<Arc<ProcessRootGuard>> {
        self.per_process
            .then(|| ProcessRootGuard::acquire(&self.path))
    }
}

/// Guards of live per-process roots. A guard is created, and an unused root is
/// removed, only under this lock.
static PROCESS_ROOTS: Mutex<Vec<(PathBuf, Weak<ProcessRootGuard>)>> = Mutex::new(Vec::new());

/// Keeps a per-process spool root while a spool of this process may use it.
pub struct ProcessRootGuard(PathBuf);

impl ProcessRootGuard {
    fn acquire(path: &Path) -> Arc<Self> {
        let mut roots = PROCESS_ROOTS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(live) = roots
            .iter()
            .filter(|(root, _)| root == path)
            .find_map(|(_, guard)| guard.upgrade())
        {
            return live;
        }
        roots.retain(|(root, _)| root != path);
        let guard = Arc::new(Self(path.to_owned()));
        roots.push((path.to_owned(), Arc::downgrade(&guard)));
        guard
    }
}

impl Drop for ProcessRootGuard {
    fn drop(&mut self) {
        let mut roots = PROCESS_ROOTS.lock().unwrap_or_else(PoisonError::into_inner);
        if roots
            .iter()
            .any(|(root, guard)| *root == self.0 && guard.strong_count() != 0)
        {
            // A newer guard keeps the root for its own holders.
            return;
        }
        roots.retain(|(root, _)| *root != self.0);
        // Spools are anonymous (Linux) or deleted on close (Windows), so the root is
        // empty once its last spool closed. `remove_dir` removes only an empty
        // directory, never a link target; a root it cannot remove is kept.
        let _ = std::fs::remove_dir(&self.0);
    }
}
