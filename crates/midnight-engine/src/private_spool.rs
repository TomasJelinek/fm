//! Per-user private roots for generated-content spools. Diagnostic reports (size
//! warnings, tree listing and presence errors) and File List output all choose
//! their root with `private_spool_roots` and open spools in it with
//! `create_private_spool`. The platform modules supply only this session's
//! bases; the candidate order is shared. Opening roots and spools is native I/O
//! for backend workers.
use midnight_worker_platform::spool::PrivateSpoolDirectory;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError, Weak},
};

#[cfg(target_os = "linux")]
#[path = "private_spool_linux.rs"]
mod session;
#[cfg(windows)]
#[path = "private_spool_windows.rs"]
mod session;

#[cfg(test)]
#[path = "private_spool_tests.rs"]
mod tests;

/// The common path component limit (ext4 bytes, NTFS UTF-16 units).
const MAX_COMPONENT: usize = 255;

/// One candidate private spool root (see `private_spool_roots`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpoolRoot {
    pub(crate) path: PathBuf,
    /// Only this process uses the root, so it is removed after its last spool
    /// closes instead of accumulating one directory per run.
    pub(crate) per_process: bool,
}

/// What a spool holds. Only whether a disk-backed root comes first depends on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpoolContent {
    /// Diagnostic reports stay small, so the session's runtime directory comes first.
    Reports,
    /// Generated output that can be large, such as a file list: the user's
    /// disk-backed cache precedes the runtime directory, a tmpfs that is sized
    /// for small runtime files.
    Large,
}

/// This session's per-user bases, gathered by the platform module. Bases that
/// are not absolute paths are ignored.
struct Session {
    /// The application's directory name and name prefix on this platform.
    application: &'static str,
    /// The user's disk-backed cache directory (Linux `$XDG_CACHE_HOME`).
    cache: Option<PathBuf>,
    /// The session's owner-only runtime directory (Linux `$XDG_RUNTIME_DIR`).
    runtime: Option<PathBuf>,
    /// A file-name-safe, lossless label of this account for names in the
    /// temporary directory, which other accounts may share.
    account: Option<String>,
    temporary: PathBuf,
}

/// This user's private roots for the spool `name`, in preference order. All
/// generated-content spools share this policy. Linux uses the XDG directories
/// first: for large content the disk-backed cache, then the session's owner-only
/// `$XDG_RUNTIME_DIR`. The temporary directory (per user on Windows, usually the
/// shared `/tmp` on Linux) always comes last, with a per-user name and then a
/// per-process name. Every root must open as an owner-only directory of this user
/// that is reached without links (`PrivateSpoolDirectory::open`), so a directory
/// that another account created first, by accident or to deny storage, is
/// skipped rather than used. Open spools with `create_private_spool`, which
/// removes a per-process root after its last spool.
pub(crate) fn private_spool_roots(name: &str, content: SpoolContent) -> Vec<SpoolRoot> {
    spool_roots(&session::current(), std::process::id(), name, content)
}

fn spool_roots(
    session: &Session,
    process: u32,
    name: &str,
    content: SpoolContent,
) -> Vec<SpoolRoot> {
    let application = session.application;
    let kept = |path| SpoolRoot {
        path,
        per_process: false,
    };
    let cache = session
        .cache
        .as_ref()
        .filter(|_| content == SpoolContent::Large);
    let mut roots: Vec<_> = cache
        .into_iter()
        .chain(&session.runtime)
        .filter(|base| base.is_absolute())
        .map(|base| kept(base.join(application).join(name)))
        .collect();
    let temporary = &session.temporary;
    if temporary.is_absolute() {
        let per_user = session
            .account
            .as_ref()
            .map(|account| format!("{application}-{account}-{name}"))
            // A label too long for one path component keeps the per-process root.
            .filter(|component| component.len() <= MAX_COMPONENT);
        roots.extend(per_user.map(|component| kept(temporary.join(component))));
        roots.push(SpoolRoot {
            path: temporary.join(format!("{application}-{name}-{process}")),
            per_process: true,
        });
    }
    roots
}

/// A spool in a private root. Field order closes the spool, and the root
/// handles it pins, before the guard that removes an empty per-process root.
/// Read-only prefixes of the spool must not outlive it.
pub(crate) struct PrivateSpool<T> {
    spool: T,
    _root: Option<Arc<ProcessRoot>>,
}
impl<T> std::ops::Deref for PrivateSpool<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.spool
    }
}
impl<T> std::ops::DerefMut for PrivateSpool<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.spool
    }
}

/// Create one spool with `create` in the first candidate root that opens as an
/// owner-only directory of this user and accepts the spool. A refused root, or
/// one whose filesystem lacks anonymous files, yields to the next candidate; the
/// last candidate's error is returned otherwise. Backend workers only (native I/O).
pub(crate) fn create_private_spool<T>(
    roots: &[SpoolRoot],
    create: impl Fn(&PrivateSpoolDirectory) -> io::Result<T>,
) -> io::Result<PrivateSpool<T>> {
    let mut failure = io::Error::new(
        io::ErrorKind::NotFound,
        "No private per-user spool directory is available",
    );
    for root in roots {
        // The guard exists before the open, so the root cannot be removed
        // between its open and the spool creation in it.
        let guard = root.per_process.then(|| ProcessRoot::acquire(&root.path));
        match PrivateSpoolDirectory::open(&root.path).and_then(|directory| create(&directory)) {
            Ok(spool) => {
                return Ok(PrivateSpool {
                    spool,
                    _root: guard,
                });
            }
            Err(error) => {
                failure = io::Error::new(error.kind(), format!("{}: {error}", root.path.display()));
            }
        }
    }
    Err(failure)
}

/// Guards of live per-process roots. A guard is created, and an unused root is
/// removed, only under this lock, so no opener can see its root disappear.
static PROCESS_ROOTS: Mutex<Vec<(PathBuf, Weak<ProcessRoot>)>> = Mutex::new(Vec::new());

/// Keeps a per-process root while a spool of this process may use it.
struct ProcessRoot(PathBuf);
impl ProcessRoot {
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
impl Drop for ProcessRoot {
    fn drop(&mut self) {
        let mut roots = PROCESS_ROOTS.lock().unwrap_or_else(PoisonError::into_inner);
        if roots
            .iter()
            .any(|(root, guard)| *root == self.0 && guard.strong_count() != 0)
        {
            // A newer guard keeps the root for its own spool.
            return;
        }
        roots.retain(|(root, _)| *root != self.0);
        // Spools are anonymous (Linux) or deleted on close (Windows), so the root
        // is empty after its last spool closed. `remove_dir` removes only an empty
        // directory, never a link target; a root it cannot remove is kept.
        let _ = std::fs::remove_dir(&self.0);
    }
}
