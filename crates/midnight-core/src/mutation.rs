//! Backend-only, platform-specific file mutation capabilities.
use crate::{Cancellation, EntryKind, EntryMetadata, FsError};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteMode {
    Trash,
    Permanent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationKind {
    CreateFile,
    CreateDirectory,
    Rename,
    EditMetadata,
    Delete(DeleteMode),
}

#[derive(Clone, Debug)]
pub struct MutationItem {
    pub path: PathBuf,
    pub kind: EntryKind,
    pub metadata: EntryMetadata,
}

/// An identity captured on a worker before confirmation. Implementations must
/// revalidate it before mutation and never follow directory links recursively.
/// No operation may silently replace another entry or fall back from trash to
/// permanent deletion. OS limitations must be returned as errors.
pub trait PreparedMutation: Send + Sync {
    fn item(&self) -> &MutationItem;
    fn metadata_snapshot(&self, cancel: &Cancellation) -> Result<crate::MetadataSnapshot, FsError> {
        let _ = cancel;
        Err(FsError::from_io(
            "inspect metadata",
            &self.item().path,
            std::io::ErrorKind::Unsupported.into(),
        ))
    }
    /// Read-only conflict probe; the eventual no-replacement rename remains authoritative.
    fn metadata_name_available(&self, name: &OsStr, cancel: &Cancellation) -> Result<(), FsError> {
        let _ = (name, cancel);
        Ok(())
    }
    /// Snapshot child names and pin identities before any child is renamed. Implementations
    /// reject links/mount crossings and enforce MAX_METADATA_TARGETS / names-byte limits.
    fn metadata_children(
        &self,
        cancel: &Cancellation,
        max_targets: usize,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        let _ = (cancel, max_targets);
        Err(FsError::from_io(
            "inspect children",
            &self.item().path,
            std::io::ErrorKind::Unsupported.into(),
        ))
    }
    /// One combined edit; the supplied name is already derived from the raw native name.
    /// Keep the same native identity through rename and subsequent metadata changes.
    fn edit_metadata(
        &self,
        patch: &crate::MetadataPatch,
        name: Option<&OsStr>,
        cancel: &Cancellation,
    ) -> crate::MetadataOutcome {
        let _ = (patch, name, cancel);
        let mut result = crate::MetadataOutcome::unchanged(self.item().path.clone());
        result.error = Some(FsError::from_io(
            "edit metadata",
            &self.item().path,
            std::io::ErrorKind::Unsupported.into(),
        ));
        result
    }
    fn rename(&self, new_name: &OsStr, cancel: &Cancellation) -> Result<PathBuf, FsError>;
    fn delete(
        &self,
        mode: DeleteMode,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(&Path),
    ) -> Result<(), FsError>;
}

/// Blocking native operations are called only on the engine's mutation workers.
/// Preparation itself performs no rename/deletion and rejects roots and `..`.
pub trait MutationService: Send + Sync {
    /// Retain the base directory before prompting; preparation never creates anything.
    fn prepare_create(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedDirectory>, FsError> {
        let _ = cancel;
        Err(FsError::from_io(
            "prepare create directory",
            path,
            std::io::Error::from(std::io::ErrorKind::Unsupported),
        ))
    }
    fn prepare(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError>;
}

/// A pinned native base for exclusive creation. Child links are never traversed.
pub trait PreparedDirectory: Send + Sync {
    /// Accept relative normal components, reusing existing intermediate directories.
    /// The final directory must be new; errors can leave created intermediates.
    fn create(&self, relative_path: &Path, cancel: &Cancellation) -> Result<PathBuf, FsError>;

    /// Create one empty regular file exclusively below this prepared directory.
    /// Accept only a basename; never replace, truncate, or follow an existing entry.
    fn create_file(&self, name: &OsStr, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        let _ = cancel;
        Err(FsError::from_io(
            "create empty file",
            Path::new(name),
            std::io::Error::from(std::io::ErrorKind::Unsupported),
        ))
    }
}
