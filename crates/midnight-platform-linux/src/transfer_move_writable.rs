//! Owner write for one change of a folder of the moved tree. An entry leaves a folder,
//! and a folder moves into another folder (its `..` entry changes), only while that
//! folder is writable. A Move changes this only on folders of the moved tree that the
//! current user owns (the selected folder and the folders below it), never on the
//! folder that contains the selection, a folder another user owns, or a link, as
//! Delete does inside its confirmed tree. The permission is added through a retained
//! descriptor, never a path, for exactly one change, and the folder gets its own mode
//! back as soon as that change returns, wherever the folder is by then.
use super::*;
use rustix::fs::{FileType, RawMode};
use std::os::fd::AsRawFd;

/// The permission that adding or removing a folder's entries needs, besides search.
const OWNER_WRITE: RawMode = 0o200;

/// Owner write added to a folder; dropping it gives the folder its own mode back.
pub(super) struct OwnerWrite<'a> {
    folder: &'a File,
    mode: RawMode,
}

impl<'a> OwnerWrite<'a> {
    /// Adds owner write to `folder` until the guard drops, when the current user owns
    /// the folder and it lacks owner write (`lacking_owner_write`). Anything else, and a
    /// folder whose mode cannot be changed, stays as it is, so the change it was wanted
    /// for fails as it would have without it.
    pub(super) fn grant(folder: &'a File) -> Option<Self> {
        let mode = lacking_owner_write(folder)?;
        chmod_retained(folder, mode | OWNER_WRITE).ok()?;
        Some(Self { folder, mode })
    }
}

impl Drop for OwnerWrite<'_> {
    fn drop(&mut self) {
        // The folder is ours and its mode was just changed the same way.
        let _ = chmod_retained(self.folder, self.mode);
    }
}

/// The permission bits of `folder` when it is a folder that the current user owns and
/// that lacks owner write. Never for the superuser, whom permissions do not bind.
pub(super) fn lacking_owner_write(folder: &File) -> Option<RawMode> {
    let caller = rustix::process::geteuid();
    if caller.is_root() {
        return None;
    }
    let stat = rustix::fs::fstat(folder).ok()?;
    let mode = stat.st_mode & 0o7777;
    (FileType::from_raw_mode(stat.st_mode) == FileType::Directory
        && stat.st_uid == caller.as_raw()
        && mode & OWNER_WRITE == 0)
        .then_some(mode)
}

/// `fchmod` refuses the path-only descriptor that a Move retains for a folder it moves.
/// Its /proc entry names exactly that inode, never a path another process could replace.
fn chmod_retained(file: &File, mode: RawMode) -> rustix::io::Result<()> {
    rustix::fs::chmod(
        format!("/proc/self/fd/{}", file.as_raw_fd()).as_str(),
        Mode::from_raw_mode(mode),
    )
}
