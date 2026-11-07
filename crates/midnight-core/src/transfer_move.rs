//! Optional recoverable source-root capture. The engine owns the whole-batch
//! verification barrier; this primitive never claims a completed Move or unlinks data.
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
/// mutation. Invoke capture only after the engine's complete verified batch barrier.
pub trait TransferMoveCapture: Send + Sync {
    fn entry(&self) -> &TransferEntry;
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
    /// Retain the whole original root under its raw basename in private sibling recovery,
    /// never unlink it. The old parent/name remain inferable locally after restart; there
    /// is no automatic recovery catalogue or restoration service.
    /// Err means no user entry was moved; terminal outcomes describe actual mutations
    /// and are idempotent. Cancellation/restoration never replaces a new source winner.
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError>;
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
