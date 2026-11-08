//! Transitional internal provider boundary; not the independently versioned plugin ABI.
//!
//! All methods, including dropping the last lease, run on backend workers.
//! Existing native and archive routes remain in production until migration parity.
mod archive;
mod lease;
mod native;
mod registry;
pub mod tree;

pub use archive::captured_archive;
pub use lease::{MountedEntry, MountedLocation};
pub use native::{NativeProvider, native_location};
pub use registry::ProviderRegistry;

use midnight_core::{Cancellation, EntryKind, EntryMetadata, NodeToken, StoredLocation};
use midnight_plugin_host::ContentSource;
use std::{fmt, sync::Arc};

/// Provider-owned logical node. Raw names and captions never become native paths.
#[derive(Clone, Debug)]
pub struct ProviderEntry {
    pub token: NodeToken,
    pub raw_name: Arc<[u8]>,
    pub display_name: String,
    pub kind: EntryKind,
    pub metadata: EntryMetadata,
}

/// An opened location, whose mount owns the interpretation of all node tokens.
pub struct OpenedLocation {
    pub mount: Arc<dyn MountedFilesystem>,
    pub root: NodeToken,
}

/// Blocking provider resolution. A missing package never falls back to native I/O.
pub trait FilesystemProvider: Send + Sync + 'static {
    fn resolve(
        &self,
        location: &StoredLocation,
        cancel: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError>;
}

/// Browse/read capabilities of one live mount. No mutation authority is implied.
/// Tokens identify logical nodes with a stable kind for this mount's lifetime.
/// Never reuse a token for another node/kind: allocate a new token or invalidate
/// the mount. Metadata/content can change; capturing content must validate its
/// own revision, and a node reference is never immutable object identity.
pub trait MountedFilesystem: Send + Sync + 'static {
    /// Errors and cancellation invalidate this attempt, not an earlier valid listing.
    /// Stop producing entries as soon as the callback returns false.
    fn list(
        &self,
        directory: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError>;

    /// Provider root semantics; the host never parses a caption or joins `..`.
    fn parent(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<Option<NodeToken>, ProviderError>;

    /// First read contract uses the existing validated, known-length content broker.
    /// Unknown-length/sequential sources need an additive contract, not materialization.
    fn open_content(
        &self,
        _node: &NodeToken,
        _cancel: &Cancellation,
    ) -> Result<Arc<dyn ContentSource>, ProviderError> {
        Err(ProviderError::Unsupported)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderError {
    Unavailable,
    StaleReference,
    Unsupported,
    Cancelled,
    ResourceLimited,
    InvalidResponse(&'static str),
    Failed(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("Filesystem provider is unavailable."),
            Self::StaleReference => formatter.write_str("Filesystem reference is no longer valid."),
            Self::Unsupported => formatter.write_str("Filesystem capability is unavailable."),
            Self::Cancelled => formatter.write_str("Filesystem request was cancelled."),
            Self::ResourceLimited => {
                formatter.write_str("Filesystem resource budget is exhausted.")
            }
            Self::InvalidResponse(reason) => {
                write!(formatter, "Invalid filesystem response: {reason}")
            }
            Self::Failed(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for ProviderError {}

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod folder_listing_tests;
#[cfg(test)]
mod ownership_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tree_tests;
