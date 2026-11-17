//! Private, session-only Chromium state. No personal browser profile is ever reused.
//!
//! Each live session holds an exclusive lock on `session-<pid>-<time>-<n>.lock` beside its
//! profile directory. The first browser launch of a process removes sessions whose lock is
//! free (their process ended without cleanup, for example after a crash); a session written by
//! an older build without a lock file is removed only when its recorded process has ended.
use crate::store::PrivateStore;
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::{
        Once,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};
static NEXT: AtomicU64 = AtomicU64::new(1);
static SWEEP: Once = Once::new();
/// Bound on directory entries inspected by one sweep.
const SWEEP_ENTRIES: usize = 4096;

pub(super) struct Profile {
    path: PathBuf,
    session: Option<PrivateStore>,
    lock: Option<(PathBuf, File)>,
    _parent: PrivateStore,
}
impl Profile {
    pub fn new(parent: &Path) -> io::Result<Self> {
        let root = PrivateStore::open(parent)?;
        SWEEP.call_once(|| {
            sweep_stale_sessions(parent, std::process::id(), super::native::process_alive)
        });
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let name = format!(
            "session-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        // Lock first, so no sweeper can observe the session directory without a live lock.
        // Locking is best effort: without it, sweepers fall back to the recorded process ID.
        let lock_path = parent.join(format!("{name}.lock"));
        let lock = match hold_lock(&lock_path) {
            Ok(lock) => Some((lock_path, lock)),
            Err(_) => {
                let _ = fs::remove_file(&lock_path);
                None
            }
        };
        let path = parent.join(name);
        let session = match PrivateStore::open(&path) {
            Ok(session) => session,
            Err(error) => {
                if let Some((lock_path, lock)) = lock {
                    drop(lock);
                    let _ = fs::remove_file(&lock_path);
                }
                return Err(error);
            }
        };
        Ok(Self {
            path,
            session: Some(session),
            lock,
            _parent: root,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        // Release the final Windows directory pin while retaining its private parent/ancestors.
        // remove_dir_all does not follow child symlinks. Only this fresh owned session is removed.
        self.session.take();
        let removed = fs::remove_dir_all(&self.path).is_ok();
        if let Some((lock_path, lock)) = self.lock.take() {
            // A remaining directory keeps its (released) lock file for the next sweep.
            if removed {
                let _ = fs::remove_file(&lock_path);
            }
            drop(lock);
        }
    }
}

/// The recording process ID of a `session-<pid>-<time>-<n>` profile name.
fn session_pid(name: &str) -> Option<u32> {
    let mut parts = name.strip_prefix("session-")?.split('-');
    let fields = [parts.next()?, parts.next()?, parts.next()?];
    if parts.next().is_some()
        || fields
            .iter()
            .any(|field| field.is_empty() || !field.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    fields[0].parse().ok().filter(|pid| *pid != 0)
}

/// Best effort, bounded removal of profiles that no live session owns. `alive` reports
/// whether a recorded process may still run; it decides only sessions without a lock file.
fn sweep_stale_sessions(parent: &Path, current: u32, alive: impl Fn(u32) -> bool) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.take(SWEEP_ENTRIES).flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if let Some(session) = name.strip_suffix(".lock") {
            // An orphan lock whose profile is already gone.
            if session_pid(session).is_some()
                && fs::symlink_metadata(parent.join(session)).is_err()
                && lock_released(&entry.path()) == Some(true)
            {
                let _ = fs::remove_file(entry.path());
            }
            continue;
        }
        let Some(pid) = session_pid(name) else {
            continue;
        };
        // DirEntry::file_type does not follow links; only real directories are profiles.
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let lock_path = parent.join(format!("{name}.lock"));
        let stale = match lock_released(&lock_path) {
            Some(released) => released,
            None => pid != current && !alive(pid),
        };
        if stale && fs::remove_dir_all(entry.path()).is_ok() {
            let _ = fs::remove_file(&lock_path);
        }
    }
}

fn hold_lock(path: &Path) -> io::Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)?;
    lock.lock()?;
    Ok(lock)
}

/// `Some(true)` when no live session holds the lock, `Some(false)` when one does and `None`
/// when there is no lock file. Any other failure is treated as held.
fn lock_released(path: &Path) -> Option<bool> {
    match OpenOptions::new().read(true).write(true).open(path) {
        Ok(lock) => Some(lock.try_lock().is_ok()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(_) => Some(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_names_are_parsed_strictly() {
        assert_eq!(session_pid("session-42-1700000000-7"), Some(42));
        for invalid in [
            "session-0-1-1",
            "session-+4-1-1",
            "session-4-1",
            "session-4-1-1-1",
            "session-4--1",
            "session-x-1-1",
            "other-4-1-1",
        ] {
            assert_eq!(session_pid(invalid), None, "{invalid}");
        }
    }

    fn session(parent: &Path, name: &str, lock: bool) -> PathBuf {
        let path = parent.join(name);
        fs::create_dir(&path).unwrap();
        fs::write(path.join("Cookies"), b"state").unwrap();
        if lock {
            File::create(parent.join(format!("{name}.lock"))).unwrap();
        }
        path
    }

    #[test]
    fn sweep_removes_only_sessions_without_a_live_owner() {
        let parent = tempfile::tempdir().unwrap();
        let current = std::process::id();
        // Liveness is injected: a real ended process ID could be reused at any moment.
        let ended = current.wrapping_add(4);
        let alive = |pid: u32| pid != ended;
        // Released lock: its process ended without cleanup.
        let crashed = session(parent.path(), &format!("session-{current}-1-1"), true);
        // Held lock: a live session, even under a reused or foreign process ID.
        let live = session(parent.path(), &format!("session-{ended}-2-2"), true);
        let held = File::options()
            .read(true)
            .write(true)
            .open(parent.path().join(format!("session-{ended}-2-2.lock")))
            .unwrap();
        held.lock().unwrap();
        // No lock file (older build): decided by the recorded process.
        let legacy_ended = session(parent.path(), &format!("session-{ended}-3-3"), false);
        let legacy_current = session(parent.path(), &format!("session-{current}-4-4"), false);
        // Orphan lock without a profile, and unrelated entries.
        File::create(parent.path().join(format!("session-{ended}-5-5.lock"))).unwrap();
        fs::create_dir(parent.path().join("unrelated")).unwrap();
        fs::write(parent.path().join("session-notes.txt"), b"keep").unwrap();

        sweep_stale_sessions(parent.path(), current, alive);

        assert!(!crashed.exists());
        assert!(
            !parent
                .path()
                .join(format!("session-{current}-1-1.lock"))
                .exists()
        );
        assert!(live.join("Cookies").exists());
        assert!(!legacy_ended.exists());
        assert!(legacy_current.join("Cookies").exists());
        assert!(
            !parent
                .path()
                .join(format!("session-{ended}-5-5.lock"))
                .exists()
        );
        assert!(parent.path().join("unrelated").exists());
        assert!(parent.path().join("session-notes.txt").exists());
        drop(held);
    }

    #[test]
    fn a_profile_holds_its_lock_until_it_is_removed() {
        let root = tempfile::tempdir().unwrap();
        // The session parent is created private; the temporary root itself is not.
        let parent = root.path().join("web-viewer-sessions");
        let profile = Profile::new(&parent).unwrap();
        let name = profile
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let lock_path = parent.join(format!("{name}.lock"));
        assert_eq!(lock_released(&lock_path), Some(false));
        fs::write(profile.path().join("state"), b"cache").unwrap();
        let path = profile.path().to_owned();
        drop(profile);
        assert!(!path.exists());
        assert!(!lock_path.exists());
    }
}
