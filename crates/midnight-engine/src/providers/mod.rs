//! Transitional internal provider boundary; not the independently versioned plugin ABI.
//!
//! All methods, including dropping the last lease, run on backend workers.
//! Existing native and archive routes remain in production until migration parity.
mod archive;
pub(crate) mod folder_probe;
mod lease;
mod native;
mod registry;
mod root_seed;
pub mod tree;

pub use archive::captured_archive;
pub(crate) use archive::captured_archive_root;
pub use lease::{MountedEntry, MountedLocation, MountedTarget};
pub use native::{NativeCapture, NativeProvider, native_location};
pub use registry::ProviderRegistry;

use midnight_core::{
    ArchiveLocation, ArchiveNode, Cancellation, EntryKind, EntryMetadata, NodeToken,
    SizeDirectoryIdentity, StoredLocation,
};
use midnight_plugin_host::ContentSource;
use std::{
    collections::{HashMap, HashSet},
    fmt,
    hash::Hash,
    sync::Arc,
};

/// Exact backend target, never constructed from a displayed name. Resolving a
/// target supplies context only; mutation still requires its own native proof.
#[derive(Clone, Debug)]
pub enum ProviderTarget {
    Native(std::path::PathBuf),
    Archive(Box<ArchiveTarget>),
}

/// A directory location, or one member in its exact immediate parent location.
#[derive(Clone, Debug)]
pub struct ArchiveTarget {
    pub location: ArchiveLocation,
    pub member: Option<ArchiveNode>,
    /// Original listed record, including directory records. None only for an
    /// unlisted captured root/ancestor. This preserves raw identity for display
    /// projections without manufacturing a record from a navigation caption.
    pub original: Option<ArchiveNode>,
}

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
    /// Consume this reference on a worker. A last owner must explicitly finish
    /// all nested native work before returning; opaque owners remain retained.
    fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        std::mem::forget(self);
        Err(crate::copy_nested_retirement::provider_error())
    }
    fn resolve(
        &self,
        location: &StoredLocation,
        cancel: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError>;
}

/// Browse/read capabilities of one live mount. No mutation authority is implied.
/// A token identifies one logical node with a stable kind and is never reused
/// for another node or kind during this mount's lifetime: allocate a new token
/// or invalidate the mount. A token stays valid for the mount's lifetime unless
/// the mount's single lister opts in to reclamation (`retain_nodes`): a token
/// it did not retain may then fail with `StaleReference`, and a node seen again
/// later may receive a new token (a provider that kept its record emits the same
/// one). Metadata/content can change; capturing content must validate its own
/// revision, and a node reference is never immutable object identity.
pub trait MountedFilesystem: Send + Sync + 'static {
    /// A shared release does not wait for another tab. The concrete last owner
    /// must acknowledge its native/provider tails before returning success.
    fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        std::mem::forget(self);
        Err(crate::copy_nested_retirement::provider_error())
    }
    /// Give up this reference where its release cannot run: inside a retirement that
    /// is already quarantined, or after the release of a resource it guards failed.
    /// Never waits and destroys no native resource. A shared reference only gives up
    /// its count; the last owner (atomic `Arc::into_inner`) passes the archive
    /// references it holds on to their own quarantined release, frees plain memory and
    /// keeps everything else retained. The default retains the incoming Arc.
    fn release_quarantined(self: Arc<Self>) {
        std::mem::forget(self);
    }
    /// Read-only target identity for ancestor cycle detection. No resolved path
    /// or operation authority is returned. Unsupported ordinary directories keep
    /// their existing browsing semantics; directory links require an identity.
    fn browse_directory_identity(
        &self,
        _node: &NodeToken,
        _cancel: &Cancellation,
    ) -> Result<Option<SizeDirectoryIdentity>, ProviderError> {
        Err(ProviderError::Unsupported)
    }

    /// Resolve navigation/action context on a backend worker. Captions, raw names
    /// and opaque tokens must not be parsed by callers to synthesize this target.
    fn resolve_target(
        &self,
        _node: &NodeToken,
        _cancel: &Cancellation,
    ) -> Result<ProviderTarget, ProviderError> {
        Err(ProviderError::Unsupported)
    }

    /// Errors and cancellation invalidate this attempt, not an earlier valid listing.
    /// Stop producing entries as soon as the callback returns false.
    fn list(
        &self,
        directory: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError>;

    /// Folder-only listing for tree branches. An adapter that classifies entries
    /// during enumeration may omit other entries before per-entry metadata,
    /// encoding and token allocation. The default emits the complete listing, so
    /// the registry still validates every token a generic provider returns.
    fn list_folders(
        &self,
        directory: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        self.list(directory, cancel, emit)
    }

    /// `list_folders` that also returns the full watch observation of the
    /// directory from the same enumeration: the `watch::signature` multiset of
    /// every entry, files and hidden entries included, with the metadata a watch
    /// probe reads. `None` when the provider has no such observation, as by
    /// default, or would read too much metadata for it (the native adapter's
    /// bounded reads for skipped files).
    fn list_folders_observed(
        &self,
        directory: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<Option<[u8; 32]>, ProviderError> {
        self.list_folders(directory, cancel, emit).map(|()| None)
    }

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

    /// Reclamation, requested only through `MountedLocation::retain_issued` while
    /// no listing, probe or parent lookup of this mount is in flight. `keep` holds
    /// every reference the host still uses, the mount's root included: each must
    /// stay valid. The provider may forget any other token, must then report
    /// `StaleReference` for it and must never emit it again for another node or
    /// kind. The default keeps every token for the mount's lifetime.
    fn retain_nodes(&self, _keep: &HashSet<NodeToken>) -> Result<(), ProviderError> {
        Ok(())
    }

    /// Node records this mount currently retains, when it tracks them. The host
    /// counts records a provider bound without issuing (failed or stopped
    /// listings, probes) toward reclamation. None: the provider does not report.
    fn node_count(&self) -> Option<usize> {
        None
    }
}

/// Give back the spare capacity of a map that reclamation shrank to below a
/// quarter of it, keeping room to grow again without immediate reallocation.
fn release_spare_capacity<K: Eq + Hash, V>(map: &mut HashMap<K, V>) {
    if map.len() < map.capacity() / 4 {
        map.shrink_to(map.len().saturating_mul(2));
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
mod archive_taint_tests;
#[cfg(test)]
mod browse_identity_tests;
#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod folder_listing_tests;
#[cfg(test)]
mod ownership_tests;
#[cfg(test)]
mod reclaim_tests;
#[cfg(test)]
#[path = "../provider_retirement_tests.rs"]
mod retirement_tests;
#[cfg(test)]
mod target_contract_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tree_tests;
