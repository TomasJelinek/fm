//! Explicit authorization for a failed native directory navigation: one consent lists the
//! folder and, for a while, its subfolders ([`ElevatedListingSession`]).
use crate::{Cancellation, Entry, FsError};
use std::{io, path::Path};

/// Whether an authorized retry can ask the operating system for permission at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElevationAvailability {
    /// A retry starts the operating system's authorization prompt.
    Available,
    /// A retry would be refused before any prompt, for example because the installation's
    /// authorization helper is missing, altered or changeable by other accounts.
    Unavailable {
        /// One or two user-facing sentences: why permission cannot be asked for, and what
        /// makes it available.
        reason: String,
        /// The technical refusal (which file or folder, and why), for the notice's details.
        details: String,
    },
}

/// Why a listing through an [`ElevatedListingSession`] did not complete. Nothing was emitted
/// for `Ended` or `Outside`; `Failed` can follow emitted entries when the listing stopped
/// midway (cancelled, or a malformed reply).
#[derive(Clone, Debug)]
pub enum SessionListingError {
    /// The session has ended (it was idle or open too long, its helper exited or its channel
    /// failed). Only a new explicit authorization lists the folder.
    Ended,
    /// The helper refused the folder because it is not the authorized folder or inside it:
    /// it is reached through a link or mount point that leads elsewhere, or lies on another
    /// volume ([`FsError::outside_authorization`]). The session continues.
    Outside(FsError),
    /// The helper could not list the folder, for example because it does not exist or is
    /// denied even with authorization. The session continues.
    Failed(FsError),
}

/// A live authorization for one folder and the folders inside it: after the consent that
/// opened it, its helper lists them without another prompt until the session ends. It lists
/// folders only; it never opens, reads or changes a file. Dropping it ends it.
pub trait ElevatedListingSession: Send + Sync {
    /// The folder the user authorized, as it was requested.
    fn root(&self) -> &Path;

    /// Whether the session may still list; `false` once it has ended or expired.
    fn is_live(&self) -> bool;

    /// List `path`, the authorized folder or a folder inside it, as
    /// [`ElevatedDirectoryService::read_directory`] does. The helper decides containment on
    /// the folder it opened, not on the spelling of `path`. Cancellation stops emitting at
    /// once; the session survives it when the helper's reply arrives soon after.
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), SessionListingError>;
}

/// Read-only directory capability. Implementations may request OS authorization only when called.
/// This service does not replace native file-operation capture or grant mutation permissions.
pub trait ElevatedDirectoryService: Send + Sync {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError>;

    /// Like [`read_directory`](Self::read_directory), and keep the authorization for `path`
    /// and the folders inside it as a session when the implementation supports one (`None`
    /// otherwise, or when the helper could not establish one). Each call asks for
    /// authorization again.
    fn open_session(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<Option<Box<dyn ElevatedListingSession>>, FsError> {
        self.read_directory(path, cancel, emit).map(|()| None)
    }

    /// Whether [`read_directory`](Self::read_directory) could ask for authorization now,
    /// decided by the same checks it runs before asking, without starting a process or a
    /// prompt. It may read file metadata and contents, so call it from a worker thread; an
    /// implementation may remember the answer for the life of the process.
    fn availability(&self) -> ElevationAvailability {
        ElevationAvailability::Available
    }
}

/// The operation of [`FsError::outside_authorization`].
const OUTSIDE_AUTHORIZATION: &str = "authorized folder boundary";

impl FsError {
    /// `path` is not covered by the authorized folder `root` although its name lies inside
    /// it: it is reached through a link or mount point that leads elsewhere, or lies on
    /// another volume. A denial (`PermissionDenied`) whose own authorization may be asked for.
    /// The message's first line is the reason; `details`, the helper's refusal, follows.
    pub fn outside_authorization(path: &Path, root: &Path, details: &str) -> Self {
        let mut message = format!(
            "This folder is outside the authorized folder {}: it is reached through a link or \
             mount point that leads elsewhere.",
            root.display()
        );
        if !details.is_empty() {
            message.push('\n');
            message.push_str(details);
        }
        Self::from_io(
            OUTSIDE_AUTHORIZATION,
            path,
            io::Error::new(io::ErrorKind::PermissionDenied, message),
        )
    }

    /// Whether this is an [`outside_authorization`](Self::outside_authorization) refusal.
    pub fn is_outside_authorization(&self) -> bool {
        self.operation == OUTSIDE_AUTHORIZATION && self.kind == io::ErrorKind::PermissionDenied
    }
}
