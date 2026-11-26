//! Removal of managed snapshots that nothing names any more (ICON-01).
//!
//! A snapshot is removed only when all of these hold, checked under `.snapshot.lock`:
//! its name is a managed snapshot name; it is a regular file, not a link or other
//! reparse point; the settings just saved do not name it; this process has not issued
//! it (a draft in an open editor may hold it); and it was last written or reused more
//! than [`COLLECTION_GRACE`] ago (another running instance may hold it in a draft or in
//! settings it has not saved yet). Anything uncertain keeps the file.
use super::{Unlock, acquire_lock, is_snapshot_name};
use midnight_core::UserPreferences;
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Duration, SystemTime},
};

/// Unnamed snapshots younger than this stay. It bounds how long another instance can
/// hold a snapshot that the settings file does not name yet; it is not a quota.
pub const COLLECTION_GRACE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Reusing a snapshot older than this refreshes its time under the lock.
const REUSE_WITHOUT_REFRESH: Duration = Duration::from_secs(24 * 60 * 60);

/// Process-wide: picks issued by any adapter object of this process are protected until
/// the process ends, so a cancelled pick is freed by a later session's collection.
struct Registry {
    issued: BTreeSet<String>,
    /// The last completed collection's folder, saved names and issued count, so an
    /// unchanged save does not list the folder again.
    last: Option<(PathBuf, BTreeSet<String>, usize)>,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    issued: BTreeSet::new(),
    last: None,
});

fn registry() -> MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Records a snapshot that this process handed to a picker (new or reused).
pub(super) fn issued(name: &str) {
    registry().issued.insert(name.to_owned());
}

/// Whether a reused snapshot is recent enough that no collection can remove it soon.
pub(super) fn recently_modified(file: &File, now: SystemTime) -> bool {
    file.metadata()
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| {
            now.duration_since(modified)
                .is_ok_and(|age| age < REUSE_WITHOUT_REFRESH)
        })
}

/// Snapshot names that saved settings refer to: each Custom Command's icon and, for
/// commands kept for another operating system, any snapshot name in their stored record.
/// Only the file name counts, wherever the path points, so the set can only be larger.
pub fn referenced_names(preferences: &UserPreferences) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for command in preferences.user_commands.iter() {
        if let Some(name) = command
            .icon
            .as_deref()
            .and_then(Path::file_name)
            .filter(|name| is_snapshot_name(name))
            .and_then(|name| name.to_str())
        {
            names.insert(name.to_owned());
        }
    }
    for setting in preferences.foreign_settings.iter() {
        names.extend(names_in_text(&setting.stored));
    }
    names
}

fn names_in_text(text: &str) -> impl Iterator<Item = String> + '_ {
    text.match_indices(".png").filter_map(|(end, _)| {
        let start = end.checked_sub(64)?;
        let name = text.get(start..end + 4)?;
        is_snapshot_name(name.as_ref()).then(|| name.to_owned())
    })
}

/// After settings were saved: removes the folder's snapshots that `referenced` (the names
/// in the saved settings) does not contain, under the rule in this module's description.
/// The adapter supplies the pinned private folder and non-following operations:
/// `plain_file_modified` returns the modification time of a regular file and `None` for
/// a link, reparse point or anything else; `remove` removes the entry itself.
/// Returns how many snapshots were removed. A busy lock skips this collection.
pub fn collect(
    directory: &Path,
    referenced: &BTreeSet<String>,
    create_exclusive: impl FnMut(&Path) -> io::Result<File>,
    open_existing: impl FnMut(&Path) -> io::Result<Option<File>>,
    plain_file_modified: impl FnMut(&Path) -> io::Result<Option<SystemTime>>,
    remove: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<usize> {
    let key = (
        directory.to_owned(),
        referenced.clone(),
        registry().issued.len(),
    );
    if registry().last.as_ref() == Some(&key) {
        return Ok(0);
    }
    let keep = || {
        let mut keep = referenced.clone();
        keep.extend(registry().issued.iter().cloned());
        keep
    };
    let removed = match collect_unreferenced(
        directory,
        keep,
        SystemTime::now(),
        COLLECTION_GRACE,
        create_exclusive,
        open_existing,
        plain_file_modified,
        remove,
    ) {
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(0),
        result => result?,
    };
    registry().last = Some(key);
    Ok(removed)
}

/// The collection itself; `keep` is evaluated under the lock, after any publication
/// that held it has registered its snapshot.
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_unreferenced(
    directory: &Path,
    keep: impl FnOnce() -> BTreeSet<String>,
    now: SystemTime,
    grace: Duration,
    mut create_exclusive: impl FnMut(&Path) -> io::Result<File>,
    mut open_existing: impl FnMut(&Path) -> io::Result<Option<File>>,
    mut plain_file_modified: impl FnMut(&Path) -> io::Result<Option<SystemTime>>,
    mut remove: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<usize> {
    let lock = acquire_lock(directory, &mut create_exclusive, &mut open_existing)?;
    let _unlock = Unlock(&lock);
    let keep = keep();
    let mut removed = 0;
    for entry in fs::read_dir(directory)? {
        let name = entry?.file_name();
        if !is_snapshot_name(&name) {
            continue;
        }
        let Some(name) = name.to_str().filter(|name| !keep.contains(*name)) else {
            continue;
        };
        let path = directory.join(name);
        // Errors keep the entry: removal is an optimisation, never a requirement.
        let Ok(Some(modified)) = plain_file_modified(&path) else {
            continue;
        };
        if now.duration_since(modified).is_ok_and(|age| age >= grace) && remove(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
#[path = "icon_snapshot_collection_tests.rs"]
mod tests;
