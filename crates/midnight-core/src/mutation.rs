//! Backend-only, platform-specific file mutation capabilities.
use crate::{Cancellation, EntryKind, EntryMetadata, FsError};
use std::{
    ffi::OsStr,
    fmt,
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

/// Private staging that a completed Rename or Delete created next to its target
/// and could not remove afterwards. Diagnostic only: it grants no authority to
/// inspect, remove or reuse that location, and it is never a reason to retry the
/// change, which already completed. Contents are never removed recursively.
#[derive(Clone, Debug)]
pub struct MutationResidue {
    /// Where the staging directory was created.
    pub path: PathBuf,
    /// False only when, after its removal failed, the staging directory was still
    /// bound at `path` with the identity it was created with. Otherwise another
    /// program moved, removed or replaced it, or its binding could not be inspected:
    /// no location is asserted, and an entry now at `path` is not the mutation's.
    pub cleanup_unconfirmed: bool,
    /// Why the cleanup failed. Its kind describes only the cleanup, never the
    /// completed change, and must not drive conflict, cancellation or retry handling.
    pub error: FsError,
}

/// One clause describing the leftover, as in "The entry was renamed, but {residue}".
/// It claims that the folder remains at its path only when that was confirmed, and
/// otherwise claims nothing it did not observe.
impl fmt::Display for MutationResidue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.cleanup_unconfirmed {
            write!(
                f,
                "its temporary staging folder at {} could not be confirmed after cleanup \
                 ({}); another program may have moved, removed or replaced it. Anything \
                 now at that path is not part of this operation",
                self.path.display(),
                self.error.message
            )
        } else {
            write!(
                f,
                "its temporary staging folder remains at {} because it could not be \
                 removed: {}. The folder is no longer needed, but another program may \
                 have placed entries in it",
                self.path.display(),
                self.error.message
            )
        }
    }
}

/// A completed Rename: the entry is now named `destination`.
#[derive(Clone, Debug)]
#[must_use = "report the residual staging of a completed Rename"]
pub struct RenameCompletion {
    pub destination: PathBuf,
    /// Staging left behind after the rename completed; see [`MutationResidue`].
    pub residue: Option<MutationResidue>,
}

/// A completed Trash move or permanent deletion of the prepared entry.
#[derive(Clone, Debug, Default)]
#[must_use = "report the residual staging of a completed Delete"]
pub struct DeleteCompletion {
    /// Staging left behind after the deletion completed; see [`MutationResidue`].
    pub residue: Option<MutationResidue>,
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
///
/// `rename` and `delete` return `Err` only when the requested change did not
/// complete; the message may describe a restored, preserved or partly deleted
/// state. Once the change completed they return `Ok`, and a failure to remove
/// private staging afterwards is reported only as the completion's residue.
pub trait PreparedMutation: Send + Sync {
    /// Read-only discovery through the retained native identity; no recursive links or writes.
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        if !cancel.wait_if_paused() {
            return Err(FsError::from_io(
                "discover operation",
                &self.item().path,
                std::io::ErrorKind::Interrupted.into(),
            ));
        }
        if self.item().kind == EntryKind::Directory {
            return Err(FsError::from_io(
                "discover operation",
                &self.item().path,
                std::io::ErrorKind::Unsupported.into(),
            ));
        }
        visit(self.item());
        Ok(())
    }
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
    /// No-replace rename within the parent; the same name completes without change.
    fn rename(&self, new_name: &OsStr, cancel: &Cancellation) -> Result<RenameCompletion, FsError>;
    /// `progress` reports each removed entry, or the selected entry once for Trash.
    fn delete(
        &self,
        mode: DeleteMode,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(&Path),
    ) -> Result<DeleteCompletion, FsError>;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn residue(cleanup_unconfirmed: bool) -> MutationResidue {
        let path = Path::new("/data/文档/.midnight-mutation-1-2-3");
        MutationResidue {
            path: path.to_owned(),
            cleanup_unconfirmed,
            error: FsError::from_io(
                "mutate entry",
                path,
                std::io::Error::new(std::io::ErrorKind::DirectoryNotEmpty, "Directory not empty"),
            ),
        }
    }

    #[test]
    fn a_residue_claims_its_location_only_when_the_staging_binding_was_confirmed() {
        let confirmed = residue(false).to_string();
        assert!(
            confirmed.starts_with(
                "its temporary staging folder remains at /data/文档/.midnight-mutation-1-2-3 \
                 because it could not be removed: Directory not empty."
            ),
            "{confirmed}"
        );
        assert!(confirmed.contains("no longer needed"), "{confirmed}");
        // An unconfirmed binding may only have been unreadable (EACCES, EIO, ESTALE):
        // the text reports what was observed and what may have happened, nothing more.
        let unconfirmed = residue(true).to_string();
        assert_eq!(
            unconfirmed,
            "its temporary staging folder at /data/文档/.midnight-mutation-1-2-3 could not \
             be confirmed after cleanup (Directory not empty); another program may have \
             moved, removed or replaced it. Anything now at that path is not part of this \
             operation"
        );
        for claim in [
            "remains at",
            "no longer needed",
            "was moved",
            "was not removed",
        ] {
            assert!(!unconfirmed.contains(claim), "{unconfirmed}");
        }
    }
}
