//! OS adapters discover targets; the shared policy owns identities, cache and pages.
use crate::api::{ActivationTarget, ErrorCode, Failure};
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
        } else if Instant::now() >= self.deadline {
            Err(Failure::new(
                ErrorCode::TimedOut,
                "Network request timed out.",
            ))
        } else {
            Ok(())
        }
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
}
