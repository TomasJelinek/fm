//! Optional recoverable source-root capture. The engine owns the complete
//! per-file verification barrier. Explicit finalization is separate from capture.
//! All filesystem methods run only on backend workers.
use crate::{Cancellation, FsError, TransferEntry, TransferRecovery, TransferSource};
use std::{path::PathBuf, sync::Arc};

/// A fresh revision of an already-held file, independent of its former name.
/// Checks the provider's native revision without reopening a path. Timestamp
/// granularity and concurrent writers prevent a snapshot/CAS guarantee; callers
/// must independently hash every byte against the original publication digest.
pub trait TransferReadGuard: Send {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
}

pub struct PreparedTransferMove {
    pub source: Box<dyn TransferSource>,
    pub capture: Arc<dyn TransferMoveCapture>,
}

/// Retains only the selected root's native authority. Preparation performs no
/// mutation. Invoke capture only after this file's complete verified publication proof.
pub trait TransferMoveCapture: Send + Sync {
    fn entry(&self) -> &TransferEntry;
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
    /// Read-only admission check before publication. Never silently downgrade to Copy.
    fn preflight_finalization(&self, _cancel: &Cancellation) -> Result<(), FsError> {
        Err(finalization_unsupported(&self.entry().item.path))
    }
    /// After matching capture, probe existing writers and bind the exact retained object.
    /// The engine must subsequently rehash this source and destination before finalize.
    fn prepare_finalization(
        &self,
        _cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        Err(finalization_unsupported(&self.entry().item.path))
    }
    /// Retain the whole original root under its raw basename in private sibling recovery,
    /// never unlink it. The old parent/name remain inferable locally after restart; there
    /// is no automatic recovery catalogue or restoration service.
    /// Err means no user entry was moved; terminal outcomes describe actual mutations
    /// and are idempotent. Cancellation/restoration never replaces a new source winner.
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError>;
}

/// Explicit removal authority for one captured regular file. Drop never removes data.
/// Native qualification and private-namespace boundaries are provider specific;
/// revision checks are not an immutable snapshot or compare-and-swap operation.
pub trait TransferMoveFinalizer: Send {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
    /// Invoke only after this file's successful independent source/destination hashes.
    /// Idempotent actual mutation facts, including failures after removal.
    fn finalize(&mut self, cancel: &Cancellation) -> TransferMoveFinalizeOutcome;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferSourceRemoval {
    Unlinked,
    /// Normal Windows disposition accepted. All application source owners must close;
    /// external readers can postpone storage reclamation. No physical-space claim.
    DeletePending,
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
    /// Captured data remains retained; never erase it or silently claim source reclamation.
    RecoveryRequired {
        source: PathBuf,
        recovery: Vec<TransferRecovery>,
        error: FsError,
    },
}
