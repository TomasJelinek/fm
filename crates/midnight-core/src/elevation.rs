//! Explicit, one-shot authorization for a failed native directory navigation.
use crate::{Cancellation, Entry, FsError};
use std::path::Path;

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

/// Read-only directory capability. Implementations may request OS authorization only when called.
/// This service does not replace native file-operation capture or grant mutation permissions.
pub trait ElevatedDirectoryService: Send + Sync {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError>;

    /// Whether [`read_directory`](Self::read_directory) could ask for authorization now,
    /// decided by the same checks it runs before asking, without starting a process or a
    /// prompt. It may read file metadata and contents, so call it from a worker thread; an
    /// implementation may remember the answer for the life of the process.
    fn availability(&self) -> ElevationAvailability {
        ElevationAvailability::Available
    }
}
