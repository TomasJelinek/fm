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

/// Pinned file/file conflict. Preparation never mutates either namespace. The
/// source must still match the original PreparedMutation and the destination its
/// retained parent. Only explicit consent may call the native Move capability;
/// Unsupported/CrossDevice never authorize a copy or delete-first fallback.
pub struct PreparedRenameReplacement {
    pub movement: crate::PreparedTransferMove,
    pub directory: Arc<dyn crate::TransferDirectory>,
    pub target: Arc<dyn crate::TransferTarget>,
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
    /// Prepare a pinned regular-file collision for recoverable native replacement.
    /// Reject links, directories, the source itself and distinct hard-link aliases.
    fn prepare_rename_replacement(
        &self,
        new_name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<PreparedRenameReplacement, FsError> {
        let _ = (new_name, cancel);
        Err(FsError::from_io(
            "prepare Rename replacement",
            &self.item().path,
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Recoverable Rename replacement is unavailable on this filesystem.",
            ),
        ))
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
/// Preparation itself performs no mutation. Rename/delete targets reject roots
/// and `..`; creation preparation pins its base directory, including a root.
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

/// A pinned native base for exclusive creation. Entered links are never traversed.
pub trait PreparedDirectory: Send + Sync {
    /// Create an explicit absolute path or a path relative to the captured base,
    /// including parent steps outside that base. Walk entered components in order,
    /// reusing ordinary intermediate directories and creating missing ones. Do not
    /// lexically collapse an untraversed component with `..` or follow entered links.
    /// Explicit path traversal may cross volumes/mounts, unlike recursive deletion.
    /// Revalidate the captured base identity even for an absolute target. The final
    /// directory must be new; errors/cancellation can leave created intermediates.
    /// Reject empty/root-only/final-dot targets before any creation. Windows resolves
    /// root-relative and same-drive-relative paths against the captured base; another
    /// drive's ambiguous relative path requires an explicit absolute path instead.
    fn create(&self, path: &Path, cancel: &Cancellation) -> Result<PathBuf, FsError>;

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
