//! Synchronous borrowing of the admitted job's actual configured I/O permit.
use midnight_core::{Cancellation, FsError};
use midnight_work_budget::{Admission, Gate, Permit};
use std::{cell::RefCell, io, path::Path, sync::Arc};

thread_local! {
    static CONFIGURED: RefCell<Option<Arc<Gate>>> = const { RefCell::new(None) };
}

struct Restore(Option<Arc<Gate>>);
impl Drop for Restore {
    fn drop(&mut self) {
        let retired = CONFIGURED.with(|configured| configured.replace(self.0.take()));
        drop(retired);
    }
}

/// Keeps a standalone admission alive until the native call returns.
/// Inside a validated synchronous witness this contains no second permit. The
/// marker exposes no authority and does not extend that witness's lifetime.
pub struct CallPermit {
    owned: Option<Permit>,
}
impl Drop for CallPermit {
    fn drop(&mut self) {
        drop(self.owned.take());
    }
}

/// Run one synchronous backend call under an actual Shared permit from the
/// configured gate, defaulting to the process-global gate. The caller retains
/// the permit throughout the closure. No references or permits are stored in TLS;
/// nothing may rely on this context
/// after the closure returns, on another thread, or across an await.
pub fn with_transfer_io_permit<R>(
    permit: &Permit,
    operation: impl FnOnce() -> R,
) -> Result<R, FsError> {
    let configured = CONFIGURED
        .with(|configured| configured.borrow().clone())
        .unwrap_or_else(midnight_work_budget::shared_io);
    with_transfer_io_permit_from(&configured, permit, operation)
}

/// Trusted backend composition only: use the controller's actual configured
/// gate, never a new per-job budget or a user-requested admission override.
/// Production controllers pass the process-global gate; isolated fixtures may
/// supply their own configured gate. TLS retains only its identity, not capacity.
#[doc(hidden)]
pub fn with_transfer_io_permit_from<R>(
    configured: &Arc<Gate>,
    permit: &Permit,
    operation: impl FnOnce() -> R,
) -> Result<R, FsError> {
    with_gate(configured, permit, operation)
}

fn with_gate<R>(
    gate: &Arc<Gate>,
    permit: &Permit,
    operation: impl FnOnce() -> R,
) -> Result<R, FsError> {
    if !permit.belongs_to(gate) || !permit.is_shared() {
        return Err(FsError::from_io(
            "transfer I/O admission",
            Path::new(""),
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Transfer calls require a Shared permit from the configured I/O gate.",
            ),
        ));
    }
    let previous = CONFIGURED.with(|configured| configured.replace(Some(gate.clone())));
    let _restore = Restore(previous);
    Ok(operation())
}

/// Borrow the active validated synchronous witness, or acquire one global Shared
/// admission for a standalone call. Cancellation never revokes an existing owner.
pub fn acquire_transfer_call(cancel: &Cancellation, path: &Path) -> Result<CallPermit, FsError> {
    let configured = CONFIGURED
        .with(|configured| configured.borrow().clone())
        .unwrap_or_else(midnight_work_budget::shared_io);
    acquire_with_gate(&configured, cancel, path)
}

/// Borrow an admitted job's validated Shared witness for a synchronous Move
/// call. Standalone Move callers retain their conservative Exclusive admission.
/// The marker cannot extend the lifetime of the admitted caller's permit.
pub fn acquire_transfer_exclusive_call(
    cancel: &Cancellation,
    path: &Path,
) -> Result<CallPermit, FsError> {
    let configured = CONFIGURED
        .with(|configured| configured.borrow().clone())
        .unwrap_or_else(midnight_work_budget::shared_io);
    acquire_exclusive_with_gate(&configured, cancel, path)
}

fn acquire_exclusive_with_gate(
    gate: &Arc<Gate>,
    cancel: &Cancellation,
    path: &Path,
) -> Result<CallPermit, FsError> {
    let interrupted = || {
        FsError::from_io(
            "transfer I/O admission",
            path,
            io::ErrorKind::Interrupted.into(),
        )
    };
    if cancel.is_cancelled() {
        return Err(interrupted());
    }
    if CONFIGURED.with(|configured| configured.borrow().is_some()) {
        return Ok(CallPermit { owned: None });
    }
    gate.acquire(Admission::Exclusive, || cancel.is_cancelled())
        .map(|permit| CallPermit {
            owned: Some(permit),
        })
        .ok_or_else(interrupted)
}

fn acquire_with_gate(
    gate: &Arc<Gate>,
    cancel: &Cancellation,
    path: &Path,
) -> Result<CallPermit, FsError> {
    let interrupted = || {
        FsError::from_io(
            "transfer I/O admission",
            path,
            io::ErrorKind::Interrupted.into(),
        )
    };
    if cancel.is_cancelled() {
        return Err(interrupted());
    }
    if CONFIGURED.with(|configured| configured.borrow().is_some()) {
        return Ok(CallPermit { owned: None });
    }
    gate.acquire(Admission::Shared, || cancel.is_cancelled())
        .map(|permit| CallPermit {
            owned: Some(permit),
        })
        .ok_or_else(interrupted)
}

#[cfg(test)]
#[path = "transfer_io_tests.rs"]
mod tests;
