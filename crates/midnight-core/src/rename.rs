//! Backend Rename ownership and cached inputs/receipts.
//!
//! Reading cached data performs no I/O. Native paths and names retain their
//! original units. Receipt identities and recovery locations are diagnostic
//! observations, never mutation capabilities.
use crate::{Cancellation, FsError, MutationItem};
use std::{
    ffi::OsString,
    fmt,
    num::NonZeroU64,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Preparation runs on a backend worker before preview. An error can retain
/// cleanup ownership even when no trustworthy prepared preview can be returned.
pub trait RenameService: Send + Sync {
    fn prepare(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedRename>, RenamePrepareFailure>;
}

/// A backend-owned resource whose disposal must be acknowledged before its
/// enclosing job, capture, mutation gate and outer retirement slot are released.
pub trait RenameResource: Send + Sync {
    /// Idempotent cleanup, serialized with attempt admission. Once finishing begins,
    /// no new mutating attempt may start; result-recovery queries may continue while
    /// the known handle is live. Remember acknowledged closes rather than resend them.
    /// Closed proves all owned or potentially published resources disposed. For known
    /// handles it requires CloseRename Unit, then acknowledged CloseMount, with actual
    /// callbacks and reply-buffer releases complete.
    /// Executing retains all owners until the associated actual work returns.
    /// Unconfirmed requires quarantine of the complete ownership chain, not only
    /// a provider handle, and must not become an endless cleanup retry loop.
    fn finish(&self) -> RenameFinish;
}

#[derive(Clone, Debug)]
pub enum RenameFinish {
    Closed,
    /// Identifiable associated work still holds ownership. An encoded Busy close
    /// is transient only while that actual completion remains outstanding.
    Executing,
    Unconfirmed(FsError),
}

/// One retained native preparation. Cached item access performs no I/O; attempt,
/// query and finish belong on backend workers. Query ignores user cancellation.
pub trait PreparedRename: RenameResource {
    fn item(&self) -> &MutationItem;
    fn attempt(&self, input: &RenameAttempt, cancel: &Cancellation) -> RenameState;
    fn query(&self) -> RenameState;
}

/// Preparation failed without granting a usable preview. Unconfirmed cleanup of
/// an adopted handle or uncertain Resolve/Prepare resource publication, including
/// a lost/malformed reply with no known handle, requires Some(retained). None is
/// allowed only when no owned or potentially published resource remains.
/// Attach retained ownership to the complete caller job/capture/gate/outer
/// retirement payload before reporting the error, and keep it until finish
/// acknowledges Closed. Unconfirmed quarantines that entire chain.
#[must_use = "Retained ownership must join the caller retirement payload"]
pub struct RenamePrepareFailure {
    pub error: FsError,
    pub retained: Option<Arc<dyn RenameResource>>,
}
impl fmt::Debug for RenamePrepareFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenamePrepareFailure")
            .field("error", &self.error)
            .field("has_retained", &self.retained.is_some())
            .finish()
    }
}
impl fmt::Display for RenamePrepareFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}
impl std::error::Error for RenamePrepareFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// One immutable input. Only a conflict permits a new sequential attempt number;
/// replaying a number requires the exact same name and policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameAttempt {
    pub number: NonZeroU64,
    pub name: OsString,
    pub policy: RenamePolicy,
}

/// One provider conflict observation, distinct from the host decision version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RenameConflictToken(pub NonZeroU64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenamePolicy {
    NoReplace,
    /// Consent applies only to the latest exact target observation for this input.
    RecoverableCurrentEntry {
        conflict: RenameConflictToken,
    },
}

#[derive(Clone, Debug)]
pub struct RenameState {
    /// Zero before an input exists: Ready or host-only Unconfirmed. Provider wire
    /// states still require zero only for Ready; accepted inputs use nonzero IDs.
    pub attempt: u64,
    pub outcome: RenameOutcome,
}

/// Metadata cached from the retained target observation. Its observed spelling
/// may differ from the requested basename and must not replace the original input.
#[derive(Clone, Debug)]
pub struct RenameConflict {
    pub token: RenameConflictToken,
    pub target: MutationItem,
    pub can_replace: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameRecoveryRole {
    Source,
    DisplacedTarget,
    EmptyStage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameSourceLocation {
    OriginalName,
    Destination,
    Recovery,
    Unknown,
}

/// A provider-reported exact residual path and opaque native identity. The host
/// does not independently verify this observation. This diagnostic receipt grants
/// no authority to open, alter or remove the named object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameRecovery {
    pub role: RenameRecoveryRole,
    pub path: PathBuf,
    pub identity: Vec<u8>,
}

/// Facts from the prepared operation. Known terminal facts remain authoritative
/// after late cancellation or cleanup failure; preserve a confirmed Renamed result.
#[derive(Clone, Debug)]
pub enum RenameOutcome {
    Ready,
    Executing,
    Conflict(RenameConflict),
    Unchanged(FsError),
    Renamed {
        name: OsString,
        same_entry: bool,
        recovery: Vec<RenameRecovery>,
        durability_confirmed: bool,
    },
    RecoveryRequired {
        source_location: RenameSourceLocation,
        destination_name: Option<OsString>,
        recovery: Vec<RenameRecovery>,
        error: FsError,
    },
    /// The host could not establish a trustworthy terminal receipt. This blocks
    /// every new mutating dispatch. An already admitted actual handler or its
    /// receipt recovery may later establish authoritative namespace facts without
    /// reopening mutation admission; failed observations never erase known facts.
    /// Names provide diagnostic context, not assertions about current locations.
    Unconfirmed {
        /// Empty only when no input has yet supplied a requested basename.
        requested_name: OsString,
        last_known_attempt: u64,
        recovery: Vec<RenameRecovery>,
        error: FsError,
    },
}
