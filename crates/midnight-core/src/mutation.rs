//! Backend-only, platform-specific file mutation capabilities.
use crate::{Cancellation, EntryKind, EntryMetadata, FsError};
use std::{
    ffi::{OsStr, OsString},
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
    /// Failures inside the entry that the caller chose to skip
    /// ([`PreparedMutation::delete_with_decisions`]). When nonzero the entry was not
    /// deleted: what was skipped remains, with the folders that contain it, under the
    /// entry's own name. Everything else was removed.
    pub skipped: u64,
}

/// One entry that a recursive Delete could not remove, offered to its caller before
/// the deletion gives up ([`PreparedMutation::delete_with_decisions`]).
#[derive(Clone, Debug)]
pub struct DeleteEntryFailure {
    /// Why, naming the entry; a folder that its entries cannot leave (write-protected,
    /// or owned by another user) is named instead of each of its entries.
    pub error: FsError,
    /// Whether trying the same step again can succeed once its cause is gone: never
    /// for a mount boundary.
    pub can_retry: bool,
}

/// What a recursive Delete does about an entry it could not remove.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteFailureChoice {
    /// Try the same step again.
    Retry,
    /// Leave the entry (a folder with everything still in it) and go on with the rest.
    /// The folders that contain it are kept too, without being reported again.
    Skip,
    /// End the deletion. What was removed stays removed; the rest is kept.
    Cancel,
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

/// Child names not processed yet that the name snapshots on one recursive metadata
/// traversal path hold together; each name returns to the budget when its entry is taken
/// (see [`PreparedMutation::metadata_children`] and `MAX_METADATA_NAMES_BYTES`).
pub const MAX_METADATA_NAMES: usize = 100_000;

/// One directory's child names, captured for recursive metadata traversal through the
/// directory's pinned native handle before any child is opened or changed. Its entries are
/// taken in the captured order and opened and pinned one at a time, only when the traversal
/// processes them, relative to the handle this retains. Dropping it closes that handle; the
/// engine drops it before it edits the directory itself.
pub trait MetadataChildren: Send {
    /// The captured names not taken yet, in native enumeration order (the next one first),
    /// without `.` and `..`.
    fn names(&self) -> &[OsString];
    /// Takes the first of `names()`, releasing it, and opens and pins the entry now so named
    /// relative to the retained directory handle, never through a pathname. A link at that
    /// name is pinned as the link and never followed; an entry on another mount (on Windows,
    /// another volume) is refused. The entry found now is pinned under its current identity,
    /// even if it replaced the captured one; a missing entry fails with `NotFound`. A
    /// failure concerns only this entry and gives its name back. `None` once every name was
    /// taken.
    fn pin_next(
        &mut self,
        cancel: &Cancellation,
    ) -> Option<Result<Arc<dyn PreparedMutation>, (OsString, FsError)>>;
}

/// Storage for [`MetadataChildren::names`]: captured names taken in order. The slots of
/// taken names are dropped, and the list shrunk, once they are at least as many as the
/// names left, so the list never holds more than twice as many slots as names not taken
/// yet, and no name or list keeps spare capacity.
#[derive(Debug)]
pub struct CapturedNames {
    names: Vec<OsString>,
    /// Leading slots whose names were taken; they hold no name.
    taken: usize,
}

impl CapturedNames {
    pub fn new(mut names: Vec<OsString>) -> Self {
        for name in &mut names {
            name.shrink_to_fit();
        }
        names.shrink_to_fit();
        Self { names, taken: 0 }
    }

    /// The names not taken yet, the next one first.
    pub fn remaining(&self) -> &[OsString] {
        &self.names[self.taken..]
    }

    /// Takes the next name, or `None` once every name was taken. Dropping the taken slots
    /// moves the names left, which is at most as many moves as names taken since the last
    /// drop: constant time per name on average.
    pub fn take_next(&mut self) -> Option<OsString> {
        let name = std::mem::take(self.names.get_mut(self.taken)?);
        self.taken += 1;
        if self.taken >= self.names.len() - self.taken {
            self.names.drain(..self.taken);
            self.names.shrink_to_fit();
            self.taken = 0;
        }
        Some(name)
    }
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
    /// Revalidates this directory and captures its child names for recursive metadata
    /// traversal, before any child is opened, pinned or changed; a link or mount crossing is
    /// never listed. A directory with more than `max_names` children, or whose names together
    /// exceed `max_bytes` (each name counted as its `OsStr::len`), fails with
    /// `std::io::ErrorKind::Other` before more names are retained; callers pass what is left
    /// of their traversal budget (at most `MAX_METADATA_NAMES` and `MAX_METADATA_NAMES_BYTES`).
    /// Every other failure uses another kind: the engine reports `Other` as a refusal of that
    /// budget. Names are released as their entries are taken (see [`MetadataChildren`]).
    fn metadata_children(
        &self,
        cancel: &Cancellation,
        max_names: usize,
        max_bytes: usize,
    ) -> Result<Box<dyn MetadataChildren>, FsError> {
        let _ = (cancel, max_names, max_bytes);
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
    /// `delete` that offers each entry it cannot remove to `failed`, which may wait for
    /// the user, instead of ending at the first one: Retry tries that step again, Skip
    /// leaves the entry and goes on with the rest of the tree (counted in
    /// [`DeleteCompletion::skipped`]), and Cancel ends the deletion as cancellation
    /// does. A folder that its entries cannot leave is offered once, for the folder.
    /// `Err` is a failure that was not offered (preparing the deletion, say) or
    /// cancellation. Without per-entry decisions a provider ends at the first failure,
    /// which its caller may then offer for the whole selected entry.
    fn delete_with_decisions(
        &self,
        mode: DeleteMode,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(&Path),
        failed: &mut dyn FnMut(DeleteEntryFailure) -> DeleteFailureChoice,
    ) -> Result<DeleteCompletion, FsError> {
        let _ = failed;
        self.delete(mode, cancel, progress)
    }
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

    /// The pinned base's native identity in the form `TransferDirectory::identity`
    /// reports for the same directory. A transfer capability opened for this base by
    /// path must match it before it changes anything there (a confirmed empty file
    /// replacing an existing one). Unsupported unless the provider can state it in
    /// that form; such a caller then refuses.
    fn transfer_identity(&self, cancel: &Cancellation) -> Result<crate::TransferIdentity, FsError> {
        let _ = cancel;
        Err(FsError::from_io(
            "identify folder",
            Path::new(""),
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "This folder cannot be identified for a safe replacement.",
            ),
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

    #[test]
    fn captured_names_are_taken_in_order_and_release_their_slots() {
        let all: Vec<OsString> = (0..1000)
            .map(|index| OsString::from(format!("名前{index}")))
            .collect();
        let mut spare = all.clone();
        for name in &mut spare {
            name.reserve(64);
        }
        let mut names = CapturedNames::new(spare);
        assert!(
            names
                .remaining()
                .iter()
                .all(|name| name.capacity() == name.len())
        );
        for (taken, expected) in all.iter().enumerate() {
            let left = all.len() - taken;
            assert_eq!(names.remaining(), &all[taken..]);
            assert!(names.names.len() < 2 * left, "{} slots", names.names.len());
            assert_eq!(names.names.capacity(), names.names.len());
            assert_eq!(names.take_next().as_ref(), Some(expected));
        }
        assert!(names.remaining().is_empty());
        assert_eq!(names.names.capacity(), 0, "the last name frees every slot");
        assert_eq!(names.take_next(), None);
    }
}
