//! OS adapters discover targets; the shared policy owns identities, cache and pages.
use crate::api::{ActivationTarget, ErrorCode, Failure, NetworkCredentials};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum NodeKind {
    Root = 0,
    Container = 1,
    Server = 2,
    Share = 3,
}

/// Adapter-owned exact resource identity, never a display caption or host path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub kind: NodeKind,
    pub key: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub target: Target,
    pub display: String,
}

/// Borrowed only for the current synchronous ABI call. OS callbacks must be
/// completed/drained before returning; no adapter may retain this reference.
pub struct Context<'a> {
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
    pub deadline: Instant,
}

impl Context<'_> {
    pub fn check(&self) -> Result<(), Failure> {
        if (self.cancelled)() {
            Err(Failure::new(
                ErrorCode::Cancelled,
                "Network request cancelled.",
            ))
        } else if deadline_elapsed(self.deadline) {
            Err(Failure::new(
                ErrorCode::TimedOut,
                "Network request timed out.",
            ))
        } else {
            Ok(())
        }
    }
}

fn deadline_elapsed(deadline: Instant) -> bool {
    #[cfg(all(test, windows))]
    if test_clock::expired() {
        return true;
    }
    Instant::now() >= deadline
}

/// Unit tests expire the deadlines checked on one thread at an exact point
/// instead of sleeping until a real deadline passes (used by the WNet tests).
#[cfg(all(test, windows))]
pub(crate) mod test_clock {
    use std::{
        cell::RefCell,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    thread_local! {
        static EXPIRED: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    }

    /// Deadlines checked on the calling thread count as elapsed once `flag` is set.
    pub(crate) fn follow(flag: Arc<AtomicBool>) {
        EXPIRED.with(|expired| *expired.borrow_mut() = Some(flag));
    }

    pub(super) fn expired() -> bool {
        EXPIRED.with(|expired| {
            expired
                .borrow()
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::SeqCst))
        })
    }
}

pub trait Adapter: Send + Sync + 'static {
    type Cursor: Send + 'static;
    fn root() -> Record;
    fn validate(target: &Target) -> Result<(), Failure>;
    /// Parse explicit user input without connecting or enumerating.
    fn manual(input: &str) -> Result<Record, Failure>;
    fn begin(target: &Target, context: &Context<'_>) -> Result<Self::Cursor, Failure>;
    fn next(cursor: &mut Self::Cursor, context: &Context<'_>) -> Result<Option<Record>, Failure>;
    /// Explicit activation only. WithinProvider carries the unchanged adapter key;
    /// shared policy replaces it with the retained complete provider address.
    fn activate(target: &Target, context: &Context<'_>) -> Result<ActivationTarget, Failure>;
    /// Explicit retry of the selected target. Credentials are borrowed for this call only.
    fn sign_in(
        _: &Target,
        _: &NetworkCredentials,
        _: &Context<'_>,
    ) -> Result<ActivationTarget, Failure> {
        Err(Failure::new(
            ErrorCode::Unsupported,
            "Network sign-in is unavailable.",
        ))
    }
    /// The server for which a successful sign-in to `target` is remembered for the rest of
    /// the session (`catalog::sign_ins`): a later explicit activation of a target with the
    /// same scope offers those credentials to its password challenge. None remembers nothing.
    fn sign_in_scope(_: &Target) -> Option<Vec<u8>> {
        None
    }
}
