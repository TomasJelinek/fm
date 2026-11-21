//! A native listing whose path names a file instead of a folder. After the listing
//! failed, its worker asks the provider what the path names, so that the path bar
//! can show that file in its folder (`Event::ListingNotAFolder`).
use crate::{Outbox, hidden_entries};
use midnight_core::{
    Cancellation, DirectoryService, Entry, EntryId, EntryKind, Event, FsError, NavigationId, PaneId,
};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

/// What the path of a failed listing names when it is not a folder.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct NamedEntry {
    /// The folder that lists the entry.
    pub(crate) parent: PathBuf,
    /// Show hidden files leaves the entry out of listings.
    pub(crate) hidden: bool,
}

/// `Some` when the listing of the absolute `path` failed because the path names an
/// existing entry that is not a folder: a file, a link to one or a special file.
/// Links are followed, as by the listing, so a link to a folder is a folder. Only a
/// "not a directory" failure asks the provider: a missing, denied or unreachable
/// path costs no further request. Call on the listing worker.
pub(crate) fn inspect(
    filesystem: &dyn DirectoryService,
    path: &Path,
    error: &FsError,
    cancel: &Cancellation,
) -> Option<NamedEntry> {
    if error.kind != ErrorKind::NotADirectory || !path.is_absolute() || cancel.is_cancelled() {
        return None;
    }
    path.file_name()?;
    let parent = path.parent().filter(|parent| parent.is_absolute())?;
    // `None` is a successfully inspected entry that is not a folder.
    if !matches!(filesystem.browse_directory_identity(path, cancel), Ok(None))
        || cancel.is_cancelled()
    {
        return None;
    }
    let mut entry = Entry::new(EntryId(0), path.to_owned(), EntryKind::File);
    if let Ok(metadata) = filesystem.entry_metadata(path) {
        entry.metadata = metadata;
    }
    Some(NamedEntry {
        parent: parent.to_owned(),
        hidden: hidden_entries::is_hidden(&entry),
    })
}

/// Before the failure of the listing of `path` is sent: say what the path names when
/// it is an entry that is not a folder. False when the output refused that event (the
/// listing was cancelled or the UI is gone); the failure is then not sent either.
pub(crate) fn report(
    filesystem: &dyn DirectoryService,
    outbox: &Outbox,
    (pane, navigation): (PaneId, NavigationId),
    path: &Path,
    error: &FsError,
    cancel: &Cancellation,
) -> bool {
    let Some(named) = inspect(filesystem, path, error, cancel) else {
        return true;
    };
    outbox.blocking_send(
        Event::ListingNotAFolder {
            pane,
            navigation,
            parent: named.parent,
            entry: path.to_owned(),
            hidden: named.hidden,
        },
        cancel,
    )
}
