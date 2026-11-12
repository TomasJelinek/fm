//! Explicit, one-shot authorization for a failed native directory navigation.
use crate::{Cancellation, Entry, FsError};
use std::path::Path;

/// Read-only directory capability. Implementations may request OS authorization only when called.
/// This service does not replace native file-operation capture or grant mutation permissions.
pub trait ElevatedDirectoryService: Send + Sync {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError>;
}
