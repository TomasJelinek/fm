//! Trusted host bridge for an already-armed backend request; never a plugin callback API.
use std::{any::Any, fmt};

/// Existing session admission credit only. The production implementation contains no policy,
/// capture or native service; unconfirmed checked cleanup may retain this credit after close.
pub trait ExternalCompareSessionCredit: Send + Sync + 'static {}

/// Backend-only construction contract, not a security seal against untrusted host code.
///
/// The payload MUST already own checked-discard protection before entering this bridge.
/// Its Drop must schedule checked cleanup or retain unconfirmed ownership safely on ANY thread,
/// including when an enclosing Command or returned send/result future has never been polled.
/// Only the Engine's private armed owner implements this in production. No raw capture,
/// validation callback or unprotected native destructor may implement this contract.
#[doc(hidden)]
pub trait ExternalCompareOwnedPayload: Any + Send {
    /// Inspect the dynamic type before consuming a known conversion; unknown owners stay intact.
    fn as_any(&self) -> &(dyn Any + Send);
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;
}

/// Non-Clone ownership of an already-armed backend request. Native cleanup remains protected
/// when command admission refuses it, when a future is unpolled, and during shutdown.
pub struct ExternalCompareRequest {
    owner: Box<dyn ExternalCompareOwnedPayload>,
}

impl ExternalCompareRequest {
    /// Trusted backend construction only: protection must precede this call.
    #[doc(hidden)]
    pub fn from_backend_owner(owner: Box<dyn ExternalCompareOwnedPayload>) -> Self {
        Self { owner }
    }

    /// Return the still-protected owner. Inspect as_any before consuming a known downcast.
    #[doc(hidden)]
    pub fn into_backend_owner(self) -> Box<dyn ExternalCompareOwnedPayload> {
        self.owner
    }
}

impl fmt::Debug for ExternalCompareRequest {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("ExternalCompareRequest")
            .finish_non_exhaustive()
    }
}
