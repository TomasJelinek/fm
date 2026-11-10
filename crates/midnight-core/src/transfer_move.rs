//! Native namespace Move and verified cross-volume source removal. Native rename
//! reads no contents. The engine owns the selected verification policy before any
//! copy-based source deletion; every pre-delete staging failure must restore.
//! All filesystem methods run only on backend workers.
use crate::{
    Cancellation, FsError, TransferDirectory, TransferEntry, TransferRecovery,
    TransferReplacementPolicy, TransferSource, TransferTarget,
};
use std::{ffi::OsStr, path::PathBuf, sync::Arc};

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
    /// Remove only this held ordinary directory, after all children have completed.
    /// Recheck the no-follow identity/parent binding; the OS must enforce emptiness.
    /// No child deletion, path-following fallback or mutation on Drop is allowed.
    fn remove_empty_directory(
        &self,
        _cancel: &Cancellation,
    ) -> Result<TransferDirectoryRemoval, FsError> {
        Err(finalization_unsupported(&self.entry().item.path))
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
    /// Transfer the exact temporary-directory authority once. After restore or
    /// accepted disposition, drop ALL source/finalizer/capture handles before
    /// invoking cleanup. This ticket never owns a source file handle.
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferSourceRemoval {
    Unlinked,
    /// Windows POSIX disposition accepted. Application DELETE owners must close to
    /// unlink the staged name; external readers can postpone physical storage release.
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
