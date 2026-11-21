//! Native namespace Move and verified cross-volume source removal. Native rename
//! reads no contents. The engine owns the selected verification policy before any
//! copy-based source deletion; every pre-delete staging failure must restore.
//! All filesystem methods run only on backend workers.
use crate::{
    Cancellation, FsError, TransferDirectory, TransferEntry, TransferRecovery,
    TransferReplacementPolicy, TransferSource, TransferTarget,
};
use std::{ffi::OsStr, io, path::PathBuf, sync::Arc};

/// A fresh revision of an already-held file, independent of its former name.
/// Checks the provider's native revision without reopening a path. Timestamp
/// granularity and concurrent writers prevent a snapshot/CAS guarantee; callers
/// must independently validate source and destination under the job's captured
/// verification policy before source deletion.
pub trait TransferReadGuard: Send {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
}

pub struct PreparedTransferMove {
    pub source: Box<dyn TransferSource>,
    pub capture: Arc<dyn TransferMoveCapture>,
}

/// A native Move entry can carry either ordinary authority or authority over the
/// original directory link. The projected source of a link is always read-only.
pub enum PreparedTransferMoveEntry {
    Ordinary(PreparedTransferMove),
    DirectoryLink {
        source: Box<dyn TransferSource>,
        capture: Arc<dyn TransferDirectoryLinkMoveCapture>,
    },
}

impl PreparedTransferMoveEntry {
    /// A read-only source. A deferred directory link supplies the selected
    /// original as a descriptor until its copy projection is prepared.
    pub fn source(&self) -> &dyn TransferSource {
        match self {
            Self::Ordinary(prepared) => &*prepared.source,
            Self::DirectoryLink { source, .. } => &**source,
        }
    }

    /// A deferred projection retains the original directory link's identity and
    /// logical directory metadata. Its effective physical identity is unknown.
    pub fn has_deferred_directory_link(&self) -> bool {
        let Self::DirectoryLink { source, capture } = self else {
            return false;
        };
        let original = capture.entry();
        source.native_identity().is_none()
            && source.native_original_identity() == Some(original.identity)
            && source.entry().identity == original.identity
            && source.entry().item.path == original.item.path
            && source.entry().item.kind == crate::EntryKind::Directory
            && original.item.kind == crate::EntryKind::DirectoryLink
            && source.virtual_identity().is_none()
    }
}

/// Qualified mutation authority over the retained original link. Its entry is the
/// no-follow DirectoryLink at the selected logical path. Preparing it does not
/// mutate either the link or its referent.
pub trait TransferDirectoryLinkMoveCapture: Send + Sync {
    /// Resolve the retained original into a read-only Copy source. Called at
    /// copy fallback or when a consented merge requires pending-source geometry.
    /// Establish the original binding and a qualified physical identity before
    /// the returned source can enumerate or read.
    fn prepare_copy_projection(
        &self,
        _cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Err(FsError {
            operation: "prepare Move link projection",
            path: self.entry().item.path.clone(),
            kind: io::ErrorKind::Unsupported,
            message: "The directory link has no deferred Copy projection.".to_owned(),
        })
    }

    /// Rename the retained link itself without opening or moving the referent.
    /// Preserve the raw link representation, including destination-relative resolution
    /// of a relative payload. Honor the supplied replacement policy and target pin
    /// or refuse before mutation. None requires no replacement.
    ///
    /// Err, CrossDevice and Unsupported guarantee no namespace change. Report
    /// a commit or recovery explicitly after any mutation, including late cancellation.
    fn try_native_move(
        &self,
        _destination: &dyn TransferDirectory,
        _name: &OsStr,
        _replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        _cancel: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        Ok(TransferNativeMoveOutcome::Unsupported)
    }

    /// `try_native_move`, except that a provider may leave saving a committed rename to the
    /// returned confirmation, which the engine runs later together with others
    /// (`TransferDurabilityConfirmation`); a `Moved` outcome with one is counted by its
    /// result alone. The default saves at once: it is `try_native_move` and none.
    fn try_native_move_deferred(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<DeferredNativeMove, FsError> {
        self.try_native_move(destination, name, replace, cancel)
            .map(|outcome| (outcome, None))
    }

    fn entry(&self) -> &TransferEntry;

    /// Verify the original link kind, identity, revision and parent binding.
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;

    /// Remove only the retained original link after its complete branch has
    /// published. Recheck the no-follow original immediately before the operation.
    /// Return Err only when the attempt leaves no committed removal or recovery
    /// residue. Cleanup or recovery after a namespace change must be reported
    /// explicitly. Drop never removes or moves the link.
    fn remove_original(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryLinkRemoval, FsError>;
}

/// Actual mutation facts after an attempted original-link removal.
#[derive(Clone, Debug)]
pub enum TransferDirectoryLinkRemoval {
    Removed {
        state: TransferSourceRemoval,
        durability_confirmed: bool,
        recovery: Vec<TransferRecovery>,
        error: Option<FsError>,
    },
    RecoveryRequired {
        recovery: Vec<TransferRecovery>,
        error: FsError,
    },
}

/// Retains one selected root or discovered child's native authority. Preparation performs no
/// mutation. Invoke capture only after this file's complete verified publication proof.
pub trait TransferMoveCapture: Send + Sync {
    /// Move the retained ordinary object into the pinned destination without reading
    /// its contents. `None` requires no replacement. Any replacement must honor the
    /// exact supplied policy and still-valid target authority. A native provider
    /// must not return `Unsupported` after qualifying this capability: same-volume
    /// permission/feature failures are errors, never permission to copy instead.
    ///
    /// `Err`, `CrossDevice` and `Unsupported` guarantee that the namespace is unchanged.
    /// After any mutation, return an explicit committed/recovery outcome, including
    /// when cancellation or a durability error arrives after the native commit.
    fn try_native_move(
        &self,
        _destination: &dyn TransferDirectory,
        _name: &OsStr,
        _replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        _cancel: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        Ok(TransferNativeMoveOutcome::Unsupported)
    }
    /// `try_native_move`, except that a provider may leave saving a committed rename to the
    /// returned confirmation, which the engine runs later together with others
    /// (`TransferDurabilityConfirmation`); a `Moved` outcome with one is counted by its
    /// result alone. The default saves at once: it is `try_native_move` and none.
    fn try_native_move_deferred(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<DeferredNativeMove, FsError> {
        self.try_native_move(destination, name, replace, cancel)
            .map(|outcome| (outcome, None))
    }
    /// Remove only this held ordinary directory, after all children have completed.
    /// Recheck the no-follow identity/parent binding; the OS must enforce emptiness.
    /// No child deletion, path-following fallback or mutation on Drop is allowed.
    fn remove_empty_directory(
        &self,
        _cancel: &Cancellation,
    ) -> Result<TransferDirectoryRemoval, FsError> {
        Err(finalization_unsupported(&self.entry().item.path))
    }
    /// After `remove_empty_directory` reported `TransferSourceRemoval::DeletePending`:
    /// the confirmation that completes it once the directory's handles are closed (see
    /// `TransferDurabilityConfirmation`). Taken once; the default has none.
    fn take_removal_confirmation(&self) -> Option<Box<dyn TransferDurabilityConfirmation>> {
        None
    }
    fn entry(&self) -> &TransferEntry;
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
    /// Read-only admission check before publication. Never silently downgrade to Copy.
    fn preflight_finalization(&self, _cancel: &Cancellation) -> Result<(), FsError> {
        Err(finalization_unsupported(&self.entry().item.path))
    }
    /// After matching capture, probe existing writers and bind the exact retained object.
    /// The engine must subsequently validate this source and destination using the
    /// captured verification policy before finalize.
    fn prepare_finalization(
        &self,
        _cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        Err(finalization_unsupported(&self.entry().item.path))
    }
    /// Temporarily stage the original under its raw basename in a private sibling
    /// directory. This method never unlinks it. Before deletion, every failure or
    /// cancellation must invoke restore; successful deletion must clean the stage.
    /// Err means no user entry was moved; terminal outcomes describe actual mutations
    /// and are idempotent. Cancellation/restoration never replaces a new source winner.
    /// An Err may keep the empty private directory of a failed attempt that its own
    /// namespace turn could not remove (for example because another program created
    /// an entry in it, or granted peers refused the turn of a repeated capture that
    /// reused it): the next capture reuses it, and otherwise `take_stage_cleanup`
    /// returns it once.
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError>;
    /// Restore the held original to its original basename without replacement.
    /// Uses an independent cleanup token: user cancellation must not cancel restore.
    /// Idempotent before capture and after successful restore. On failure the
    /// original remains in the reported temporary location; never erase it.
    fn restore(&self) -> Result<(), FsError> {
        Err(finalization_unsupported(&self.entry().item.path))
    }
    /// The no-replace restoration completed, even if a subsequent durability check
    /// returned an error. This records the provider's namespace action, not a
    /// promise against later external mutation. Default providers report no fact.
    fn restoration_completed(&self) -> bool {
        false
    }
    /// Transfer the exact temporary-directory authority once: the stage of the
    /// capture, or the empty one that a failed capture kept (see `capture`). After
    /// restore or accepted disposition, drop ALL source/finalizer/capture handles
    /// before invoking cleanup. This ticket never owns a source file handle.
    fn take_stage_cleanup(&self) -> Option<Box<dyn TransferMoveStageCleanup>> {
        None
    }
}

/// Namespace facts from an optional native Move capability. No outcome asserts
/// that file contents were read, hashed, copied or independently verified.
#[derive(Clone, Debug)]
pub enum TransferNativeMoveOutcome {
    Moved {
        destination: PathBuf,
        recovery: Vec<TransferRecovery>,
        durability_confirmed: bool,
    },
    /// The exact same parent and raw basename already name this object. A distinct
    /// hard-link alias is a conflict, not a completed move or permission to unlink.
    SameEntry,
    /// Native volume mismatch/EXDEV; no source or target namespace change occurred.
    CrossDevice,
    /// Legacy provider lacks this capability; no mutation occurred.
    Unsupported,
    RecoveryRequired {
        /// Present only if the original source reached its requested destination.
        destination: Option<PathBuf>,
        recovery: Vec<TransferRecovery>,
        error: FsError,
    },
}

/// Empty-directory-only cleanup. Neither Drop nor cleanup may delete a child.
pub trait TransferMoveStageCleanup: Send {
    fn path(&self) -> &std::path::Path;
    fn cleanup(self: Box<Self>) -> Result<(), FsError>;
}

/// Explicit removal authority for one captured regular file. Drop never removes data.
/// Native qualification and private-namespace boundaries are provider specific;
/// revision checks are not an immutable snapshot or compare-and-swap operation.
pub trait TransferMoveFinalizer: Send {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
    /// Invoke only after successful independent source/destination validation under
    /// the immutable job policy (full SHA-256 or size and sampled content).
    /// Idempotent actual mutation facts, including failures after removal.
    fn finalize(&mut self, cancel: &Cancellation) -> TransferMoveFinalizeOutcome;
    /// After `finalize` reported a `TransferSourceRemoval::DeletePending` removal: the
    /// confirmation that completes it once the source's handles are closed (see
    /// `TransferDurabilityConfirmation`). Taken once; the default has none.
    fn take_removal_confirmation(&mut self) -> Option<Box<dyn TransferDurabilityConfirmation>> {
        None
    }
}

/// A native Move's outcome and, when the provider left saving the rename for later, its
/// confirmation (`try_native_move_deferred`).
pub type DeferredNativeMove = (
    TransferNativeMoveOutcome,
    Option<Box<dyn TransferDurabilityConfirmation>>,
);

/// The part of a committed source change (a native rename, a removal) that a provider
/// completes later: saving the folders it changed to disk and, for a removal by delete
/// disposition, checking that its name is gone, which Windows does only when the handle
/// that set the disposition closes. It is run once: a removal's after the engine dropped
/// every source, capture and finalizer of the removed entry, a deferred rename's together
/// with the job's other pending ones (before the job removes a folder, and at its end).
/// The engine counts the change only then.
pub trait TransferDurabilityConfirmation: Send {
    /// True only when the change is saved: for a removal, its name is verifiably gone from
    /// the folder that held it (it is then `Unlinked`) and that folder is saved. False
    /// leaves the change unconfirmed (a removal stays `DeletePending`). Each folder is
    /// saved at most once per `round`, whose confirmations all run after their changes.
    fn confirm(self: Box<Self>, round: &mut TransferDurabilityRound) -> bool;
}

/// Confirmations the engine runs together, every one after the change it confirms was
/// committed: a folder saved for one of them is saved for all of them.
#[derive(Debug, Default)]
pub struct TransferDurabilityRound {
    saved: std::collections::HashMap<crate::TransferIdentity, bool>,
}
impl TransferDurabilityRound {
    /// Whether `folder` is saved in this round: `save` runs for its first request only.
    pub fn folder(&mut self, folder: crate::TransferIdentity, save: impl FnOnce() -> bool) -> bool {
        *self.saved.entry(folder).or_insert_with(save)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferSourceRemoval {
    Unlinked,
    /// Windows POSIX disposition accepted. Application DELETE owners must close to
    /// unlink the staged name; external readers can postpone physical storage release.
    /// A provider can hand over a `TransferDurabilityConfirmation` that checks the
    /// unlink once those owners are closed.
    DeletePending,
}

/// Actual empty-directory namespace disposition, independent of later sync errors.
pub struct TransferDirectoryRemoval {
    pub state: TransferSourceRemoval,
    pub durability_confirmed: bool,
}

#[derive(Clone, Debug)]
pub enum TransferMoveFinalizeOutcome {
    Removed {
        source: PathBuf,
        recovery: TransferRecovery,
        state: TransferSourceRemoval,
        durability_confirmed: bool,
    },
    Retained {
        source: PathBuf,
        recovery: TransferRecovery,
        error: FsError,
    },
}

fn finalization_unsupported(path: &std::path::Path) -> FsError {
    FsError::from_io(
        "Move finalization",
        path,
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "verified source finalization is unsupported; choose Copy to retain the source",
        ),
    )
}

#[derive(Clone, Debug)]
pub enum TransferMoveCaptureOutcome {
    /// Only root namespace capture completed; file verification is the engine's duty.
    Captured {
        source: PathBuf,
        recovery: TransferRecovery,
    },
    /// A captured entry was restored without replacement. It may have been a raced
    /// unexpected entry; this is not a claim that the selected original was restored.
    Restored { source: PathBuf, error: FsError },
    /// Exceptional staged data remains; attempt explicit no-replace restoration.
    /// Never erase it or silently claim source reclamation.
    RecoveryRequired {
        source: PathBuf,
        recovery: Vec<TransferRecovery>,
        error: FsError,
    },
}
