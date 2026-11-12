//! Typed discovery locations. These are never native paths or file-operation targets.
use crate::Cancellation;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkAddress {
    pub provider: String,
    pub opaque: Arc<[u8]>,
    /// Presentation only. Identity and restoration use `opaque`.
    pub display: String,
}
impl NetworkAddress {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.provider.is_empty()
            || self.provider.len() > 128
            || !self
                .provider
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
            || self.opaque.is_empty()
            || self.opaque.len() > 16 * 1024 * 1024
            || self.display.len() > 16 * 1024
            || self.display.contains('\0')
        {
            return Err("Invalid Network Neighborhood address.");
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct NetworkNode {
    pub address: NetworkAddress,
    pub display: String,
}

/// Stable backend states; presentation text must never decide recovery behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NetworkFailureKind {
    Invalid,
    Unsupported,
    NotFound,
    PermissionDenied,
    SourceChanged,
    Cancelled,
    ResourceLimit,
    Io,
    StaleReference,
    Busy,
    Unavailable,
    Offline,
    AuthenticationRequired,
    MountRequired,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkFailure {
    pub kind: NetworkFailureKind,
    pub message: String,
    pub os_code: Option<i32>,
}
impl NetworkFailure {
    pub fn new(kind: NetworkFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            os_code: None,
        }
    }
}
impl std::fmt::Display for NetworkFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Recovery labels do not expose provider credential/mount diagnostics.
        // Generic I/O failures retain their useful bounded diagnostic message.
        let label = match self.kind {
            NetworkFailureKind::Invalid => "Invalid network response",
            NetworkFailureKind::Unsupported => "Unsupported network operation",
            NetworkFailureKind::NotFound => "Network target not found",
            NetworkFailureKind::PermissionDenied => "Access denied",
            NetworkFailureKind::SourceChanged => "Network target changed",
            NetworkFailureKind::Cancelled => "Network request canceled",
            NetworkFailureKind::ResourceLimit => "Network resource limit reached",
            NetworkFailureKind::Io => &self.message,
            NetworkFailureKind::StaleReference => "Network location expired",
            NetworkFailureKind::Busy => "Network discovery is busy",
            NetworkFailureKind::Unavailable => "Network service unavailable",
            NetworkFailureKind::Offline => "Offline",
            NetworkFailureKind::AuthenticationRequired => "Sign-in required",
            NetworkFailureKind::MountRequired => "Mount required",
            NetworkFailureKind::TimedOut => "Network request timed out",
        };
        formatter.write_str(label)
    }
}
impl std::error::Error for NetworkFailure {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_failure_display_has_distinct_safe_recovery_labels() {
        for (kind, label) in [
            (NetworkFailureKind::Offline, "Offline"),
            (
                NetworkFailureKind::Unavailable,
                "Network service unavailable",
            ),
            (
                NetworkFailureKind::AuthenticationRequired,
                "Sign-in required",
            ),
            (NetworkFailureKind::MountRequired, "Mount required"),
            (NetworkFailureKind::PermissionDenied, "Access denied"),
        ] {
            let error = NetworkFailure::new(kind, "provider diagnostic with private credentials");
            assert_eq!(error.to_string(), label);
        }
        let error = NetworkFailure::new(NetworkFailureKind::Io, "Provider failed with code 1234");
        assert_eq!(error.to_string(), "Provider failed with code 1234");
    }
}

#[derive(Clone, Debug)]
pub enum NetworkDestination {
    Browse(NetworkAddress),
    /// OS-reported absolute path. The native filesystem must open it successfully
    /// before the UI leaves the Network location.
    Native(PathBuf),
}

#[derive(Clone, Debug)]
pub enum NetworkAction {
    OpenRoot,
    Parse(String),
    Browse(NetworkAddress),
    Activate(NetworkAddress),
    Parent(NetworkAddress),
    Refresh(NetworkAddress),
}

#[derive(Clone, Debug)]
pub struct NetworkListing {
    pub address: NetworkAddress,
    pub parent: Option<NetworkAddress>,
    pub entries: Arc<[crate::Entry]>,
    pub index: Arc<crate::DirectoryIndex>,
}

#[derive(Clone, Debug)]
pub enum NetworkUpdate {
    Listing {
        listing: NetworkListing,
        done: bool,
        error: Option<NetworkFailure>,
    },
    Native(PathBuf),
    Failed(NetworkFailure),
}

/// Calls and resource reclamation belong on bounded backend workers.
pub trait NetworkNeighborhood: Send + Sync + std::fmt::Debug {
    fn root(&self, cancel: &Cancellation) -> Result<NetworkAddress, NetworkFailure>;
    fn parse(&self, text: &str, cancel: &Cancellation) -> Result<NetworkAddress, NetworkFailure>;
    fn parent(
        &self,
        address: &NetworkAddress,
        cancel: &Cancellation,
    ) -> Result<Option<NetworkAddress>, NetworkFailure>;
    fn list(
        &self,
        address: &NetworkAddress,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(NetworkNode) -> bool,
    ) -> Result<(), NetworkFailure>;
    fn activate(
        &self,
        address: &NetworkAddress,
        cancel: &Cancellation,
    ) -> Result<NetworkDestination, NetworkFailure>;
    fn refresh(
        &self,
        address: &NetworkAddress,
        cancel: &Cancellation,
    ) -> Result<(), NetworkFailure>;
}
