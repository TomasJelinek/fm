//! Saving the folders a committed native Move changed, so that the Move can report its
//! durability as confirmed (TRANSFERS.md, "Move durability on Windows").
//!
//! `FlushFileBuffers` on a folder writes what the filesystem holds for it. Windows accepts
//! it only through a handle that may add files or subfolders to the folder (`FILE_ADD_FILE`
//! or `FILE_ADD_SUBDIRECTORY`, the folder forms of the `FILE_WRITE_DATA` and
//! `FILE_APPEND_DATA` a flush requires); through a handle that may only read or list the
//! folder, as the Move's own pins do, it fails with ERROR_ACCESS_DENIED (both measured on
//! NTFS). Each folder is therefore reopened through the Move's retained handle, never by
//! path, with one of those rights, sharing everything, and closed again. A volume handle,
//! whose flush needs administrator rights, is never used.
use super::*;
use midnight_core::TransferNativeMoveOutcome;
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED,
};

/// Saves the folders a committed rename changed and keeps the result in a `Moved` outcome:
/// it stays confirmed only if it was and every folder is saved. Any other outcome is
/// returned as it is. Runs after the rename whatever the cancellation, since the rename
/// is committed, and outside its namespace turn, since a flush changes no name.
pub(super) fn confirm(
    mut outcome: TransferNativeMoveOutcome,
    folders: &[(&File, TransferIdentity)],
) -> TransferNativeMoveOutcome {
    if let TransferNativeMoveOutcome::Moved {
        durability_confirmed,
        ..
    } = &mut outcome
    {
        // Evaluated also when an earlier folder could not be saved: each is still flushed.
        *durability_confirmed &= saved(folders);
    }
    outcome
}

/// Flushes each of `folders` once (a folder given twice, by identity, is flushed once) and
/// says whether all of them are saved as far as Windows allows: flushed, or on a filesystem
/// or file server that does not flush folders. A folder that cannot be opened for its
/// flush, or whose flush fails, is not.
pub(super) fn saved(folders: &[(&File, TransferIdentity)]) -> bool {
    let mut flushed = Vec::with_capacity(folders.len());
    let mut all = true;
    for &(folder, identity) in folders {
        if !flushed.contains(&identity) {
            flushed.push(identity);
            all &= save(folder) != Saved::No;
        }
    }
    all
}

/// What one folder's flush established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Saved {
    Flushed,
    /// The filesystem or file server does not flush folders: nothing more can be done.
    Unsupported,
    /// The flush failed, or no handle that may flush the folder could be opened.
    No,
}

fn save(folder: &File) -> Saved {
    match reopen(folder) {
        Ok(handle) => after_flush(handle.sync_all()),
        Err(error) => after_open(&error),
    }
}

/// A new handle on the same folder that may add files to it, or else subfolders: a Move's
/// destination grants the right its rename needed, its source folder may grant only one.
fn reopen(folder: &File) -> io::Result<File> {
    let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    native::reopen(folder, FILE_ADD_FILE, share).or_else(|error| {
        if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
            native::reopen(folder, FILE_ADD_SUBDIRECTORY, share)
        } else {
            Err(error)
        }
    })
}

/// No handle that may flush the folder: unless the filesystem has no such opens, the
/// account may add nothing to the folder or another program denies write sharing on it,
/// and the change stays unconfirmed.
fn after_open(error: &io::Error) -> Saved {
    if not_supported(error) {
        Saved::Unsupported
    } else {
        Saved::No
    }
}

/// The handle may add entries to the folder, all that a flush requires of it, so a refusal
/// is the filesystem's or the file server's own: one that does not flush folders answers
/// "not supported", or ERROR_ACCESS_DENIED as a file server may. Any other error is a flush
/// that failed.
fn after_flush(result: io::Result<()>) -> Saved {
    match result {
        Ok(()) => Saved::Flushed,
        Err(error)
            if not_supported(&error)
                || error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) =>
        {
            Saved::Unsupported
        }
        Err(_) => Saved::No,
    }
}

/// ERROR_INVALID_FUNCTION (an unimplemented request), ERROR_NOT_SUPPORTED and
/// ERROR_INVALID_PARAMETER: the operation does not exist for a folder there.
fn not_supported(error: &io::Error) -> bool {
    matches!(
        error
            .raw_os_error()
            .and_then(|code| u32::try_from(code).ok()),
        Some(ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED | ERROR_INVALID_PARAMETER)
    )
}

#[cfg(test)]
#[path = "transfer_folder_flush_tests.rs"]
mod tests;
