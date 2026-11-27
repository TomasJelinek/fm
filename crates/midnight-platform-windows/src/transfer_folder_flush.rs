//! Saving the folders a committed native Move changed, so that the Move can report its
//! durability as confirmed (transfers.md, "Move durability on Windows").
//!
//! `FlushFileBuffers` on a folder writes what the filesystem holds for it. Windows accepts
//! it only through a handle that may add files or subfolders to the folder (`FILE_ADD_FILE`
//! or `FILE_ADD_SUBDIRECTORY`, the folder forms of the `FILE_WRITE_DATA` and
//! `FILE_APPEND_DATA` a flush requires); through a handle that may only read or list the
//! folder, as the Move's own pins do, it fails with ERROR_ACCESS_DENIED (both measured on
//! NTFS). Each folder is therefore reopened through the Move's retained handle, never by
//! path, with one of those rights, sharing everything, and closed again. A volume handle,
//! whose flush needs administrator rights, is never used.
//!
//! A removal by delete disposition is confirmed the same way once its name has left the
//! folder, which Windows does only when the handle that set the disposition closes
//! ([`Removal`], [`removed`]). A Move's renames leave saving their folders to the engine,
//! which saves each folder once for all the renames it confirms together ([`settle`]).
use super::*;
use midnight_core::{
    DeferredNativeMove, TransferDurabilityConfirmation, TransferDurabilityRound,
    TransferNativeMoveOutcome,
};
use windows_sys::{
    Wdk::Storage::FileSystem::FILE_OPEN,
    Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER,
        ERROR_NOT_SUPPORTED,
    },
};

/// A folder a committed change touched, held through one of the Move's own handles (never
/// reopened by path) until it is saved.
pub(super) enum Held {
    Directory(destination::Directory),
    Handle(Arc<File>, TransferIdentity),
}
impl Held {
    fn file(&self) -> &File {
        match self {
            Self::Directory(directory) => directory.native_file(),
            Self::Handle(file, _) => file.as_ref(),
        }
    }
    fn identity(&self) -> TransferIdentity {
        match self {
            Self::Directory(directory) => directory.identity(),
            Self::Handle(_, identity) => *identity,
        }
    }
}

/// The folders a committed rename changed, saved once the engine confirms the rename.
struct Renamed(Vec<Held>);
impl TransferDurabilityConfirmation for Renamed {
    fn confirm(self: Box<Self>, round: &mut TransferDurabilityRound) -> bool {
        all_saved(round, &self.0)
    }
}

/// Completes a native rename's outcome. A `Moved` outcome gets its `folders` saved: at once
/// (it stays confirmed only if it was and every folder is saved), or, `deferred`, by the
/// returned confirmation, which the engine runs together with the job's other renames so
/// that a folder they share is flushed once; the outcome then says unconfirmed until it
/// runs. Any other outcome is returned as it is, without confirmation. Runs after the
/// rename whatever the cancellation, since the rename is committed, and outside its
/// namespace turn, since a flush changes no name.
pub(super) fn settle(
    outcome: TransferNativeMoveOutcome,
    folders: Vec<Held>,
    deferred: bool,
) -> DeferredNativeMove {
    let TransferNativeMoveOutcome::Moved {
        destination,
        recovery,
        durability_confirmed,
    } = outcome
    else {
        return (outcome, None);
    };
    let (durability_confirmed, confirmation) = if deferred {
        // Unless a step of the rename already failed, the confirmation decides.
        let confirmation = durability_confirmed
            .then(|| Box::new(Renamed(folders)) as Box<dyn TransferDurabilityConfirmation>);
        (false, confirmation)
    } else {
        let saved = all_saved(&mut TransferDurabilityRound::default(), &folders);
        (durability_confirmed && saved, None)
    };
    let outcome = TransferNativeMoveOutcome::Moved {
        destination,
        recovery,
        durability_confirmed,
    };
    (outcome, confirmation)
}

/// Whether every folder is saved in `round`; each is still saved after one that is not.
fn all_saved(round: &mut TransferDurabilityRound, folders: &[Held]) -> bool {
    folders.iter().fold(true, |all, folder| {
        let saved = saved_in(round, folder.file(), folder.identity());
        all && saved
    })
}

/// A removal by delete disposition of the entry `name` in `folder`, confirmed once every
/// handle the job holds on that entry is closed (`TransferDurabilityConfirmation`; the
/// engine drops them first). Holds only the folder, never the removed entry.
pub(super) struct Removal {
    folder: destination::Directory,
    name: OsString,
}
impl Removal {
    pub(super) fn new(folder: destination::Directory, name: &OsStr) -> Self {
        Self {
            folder,
            name: name.to_owned(),
        }
    }
}
impl TransferDurabilityConfirmation for Removal {
    fn confirm(self: Box<Self>, round: &mut TransferDurabilityRound) -> bool {
        gone(self.folder.native_file(), &self.name)
            && saved_in(round, self.folder.native_file(), self.folder.identity())
    }
}

/// Whether the removal of `name` from `folder` is confirmed: the name is verifiably gone
/// from the folder, and then the folder is saved ([`saved`]). Windows takes a name removed
/// by delete disposition out of its folder only when the handle that set the disposition
/// (every duplicate of it) closes, also while other handles keep the entry open (measured
/// on NTFS for a file, a folder and a junction). Until then a lookup of the name fails with
/// ERROR_ACCESS_DENIED.
pub(super) fn removed(folder: &File, identity: TransferIdentity, name: &OsStr) -> bool {
    gone(folder, name) && saved(&[(folder, identity)])
}

/// The name is looked up through the folder's own handle, never a path. Only "no such
/// file" proves it gone: a name still pending deletion, another entry under that name and
/// every other failure do not.
fn gone(folder: &File, name: &OsStr) -> bool {
    native::open_relative(
        folder,
        name,
        FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        0,
    )
    .is_err_and(|error| error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32))
}

/// Flushes each of `folders` once (a folder given twice, by identity, is flushed once) and
/// says whether all of them are saved as far as Windows allows: flushed, or on a filesystem
/// or file server that does not flush folders. A folder that cannot be opened for its
/// flush, or whose flush fails, is not.
pub(super) fn saved(folders: &[(&File, TransferIdentity)]) -> bool {
    let mut round = TransferDurabilityRound::default();
    folders.iter().fold(true, |all, &(folder, identity)| {
        let saved = saved_in(&mut round, folder, identity);
        all && saved
    })
}

/// Whether `folder` is saved in `round`: flushed for the round's first request only.
fn saved_in(
    round: &mut TransferDurabilityRound,
    folder: &File,
    identity: TransferIdentity,
) -> bool {
    round.folder(identity, || save(folder) != Saved::No)
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
        Ok(handle) => {
            #[cfg(test)]
            FLUSHES.with(|flushes| flushes.set(flushes.get() + 1));
            after_flush(handle.sync_all())
        }
        Err(error) => after_open(&error),
    }
}

#[cfg(test)]
thread_local! {
    /// The folder flushes this thread requested, for tests that count them.
    pub(super) static FLUSHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
