//! Counts one folder's contents for Properties on a cancellable backend worker:
//! files, folders and their logical bytes. Links met inside are counted but never
//! followed, other filesystems are not entered and ancestry cycles stop.

use midnight_core::{Cancellation, EntryKind, FileSystem, FsError};
use std::{
    collections::HashSet,
    io,
    mem::size_of,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// How often `count` reports totals so far.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
/// Paths of folders waiting to be read; a wider frontier is counted as unread.
const PENDING_PATH_BUDGET: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Totals {
    pub(crate) files: u64,
    pub(crate) folders: u64,
    pub(crate) bytes: u64,
    /// Links inside the folder; none is followed.
    pub(crate) links: u64,
    /// Devices, pipes and sockets.
    pub(crate) others: u64,
    /// Entries or folders that could not be read, so their contents are missing.
    pub(crate) unreadable: u64,
    /// Folders on another filesystem (mount points), counted but not entered.
    pub(crate) other_filesystems: u64,
}

enum Pending {
    Enter(PathBuf),
    Leave(midnight_core::SizeDirectoryIdentity),
}

fn cost(path: &Path) -> usize {
    path.as_os_str()
        .as_encoded_bytes()
        .len()
        .saturating_add(size_of::<Pending>())
}

/// Count `root`'s contents (not `root` itself). `progress` receives the totals so
/// far at most every half second and returns false when nobody waits for them any
/// more. That, like cancellation, returns `Interrupted`.
pub(crate) fn count(
    filesystem: &dyn FileSystem,
    root: &Path,
    cancel: &Cancellation,
    progress: &mut dyn FnMut(&Totals) -> bool,
) -> Result<Totals, FsError> {
    let interrupted = || {
        FsError::from_io(
            "count folder contents",
            root,
            io::ErrorKind::Interrupted.into(),
        )
    };
    let device = filesystem
        .resolve_size_directory(root, cancel)?
        .ok_or_else(|| {
            FsError::from_io(
                "count folder contents",
                root,
                io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "The folder is no longer a folder",
                ),
            )
        })?
        .identity
        .device;
    let mut totals = Totals::default();
    let mut ancestors = HashSet::new();
    let mut pending = vec![Pending::Enter(root.to_owned())];
    let mut pending_bytes = cost(root);
    let mut reported = Instant::now();
    let add = |total: &mut u64, value: u64| *total = total.saturating_add(value);
    while let Some(next) = pending.pop() {
        if cancel.is_cancelled() {
            return Err(interrupted());
        }
        let path = match next {
            Pending::Leave(identity) => {
                ancestors.remove(&identity);
                continue;
            }
            Pending::Enter(path) => path,
        };
        pending_bytes = pending_bytes.saturating_sub(cost(&path));
        let directory = match filesystem.resolve_size_directory(&path, cancel) {
            Ok(Some(target)) => target,
            _ if cancel.is_cancelled() => return Err(interrupted()),
            Ok(None) | Err(_) => {
                add(&mut totals.unreadable, 1);
                continue;
            }
        };
        if directory.identity.device != device {
            add(&mut totals.other_filesystems, 1);
            continue;
        }
        // A bind mount can make a folder its own descendant.
        if !ancestors.insert(directory.identity) {
            continue;
        }
        pending.push(Pending::Leave(directory.identity));
        let mut wanted = true;
        let listed = filesystem.read_directory(&directory.path, cancel, &mut |entry| {
            match entry.kind {
                EntryKind::Directory => {
                    add(&mut totals.folders, 1);
                    let entry_cost = cost(&entry.path);
                    if pending_bytes.saturating_add(entry_cost) <= PENDING_PATH_BUDGET {
                        pending_bytes += entry_cost;
                        pending.push(Pending::Enter(entry.path));
                    } else {
                        add(&mut totals.unreadable, 1);
                    }
                }
                EntryKind::File => {
                    let length = match entry.metadata.size {
                        Some(size) if filesystem.listing_metadata_complete(&entry) => Some(size),
                        _ => filesystem
                            .entry_metadata(&entry.path)
                            .ok()
                            .and_then(|metadata| metadata.size),
                    };
                    add(&mut totals.files, 1);
                    match length {
                        Some(length) => add(&mut totals.bytes, length),
                        None => add(&mut totals.unreadable, 1),
                    }
                }
                EntryKind::Symlink | EntryKind::DirectoryLink => add(&mut totals.links, 1),
                EntryKind::Other => add(&mut totals.others, 1),
            }
            if reported.elapsed() >= PROGRESS_INTERVAL {
                reported = Instant::now();
                wanted = progress(&totals);
            }
            wanted && !cancel.is_cancelled()
        });
        if !wanted || cancel.is_cancelled() {
            return Err(interrupted());
        }
        if listed.is_err() {
            add(&mut totals.unreadable, 1);
        }
    }
    Ok(totals)
}
