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

/// Credentials for one explicit network activation. The password is wiped on
/// drop and none of the fields appear in debug output.
pub struct NetworkCredentials {
    username: String,
    domain: String,
    password: zeroize::Zeroizing<String>,
}

impl NetworkCredentials {
    /// Reject fields that cannot cross the provider's bounded UTF-8 interface.
    pub fn new(username: String, domain: String, password: String) -> Result<Self, &'static str> {
        let password = zeroize::Zeroizing::new(password);
        if username.is_empty() {
            return Err("Enter a username.");
        }
        if [username.as_str(), domain.as_str(), password.as_str()]
            .iter()
            .any(|field| field.len() > 4096 || field.contains('\0'))
        {
            return Err("Sign-in fields must be at most 4096 bytes and contain no NUL characters.");
        }
        Ok(Self {
            username,
            domain,
            password,
        })
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn domain(&self) -> &str {
        &self.domain
    }

    pub fn password(&self) -> &[u8] {
        self.password.as_bytes()
    }
}

impl std::fmt::Debug for NetworkCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NetworkCredentials(<redacted>)")
    }
}

#[derive(Clone, Debug)]
pub enum NetworkAction {
    OpenRoot,
    Parse(String),
    Browse(NetworkAddress),
    Activate(NetworkAddress),
    SignIn(NetworkAddress, Arc<NetworkCredentials>),
    Parent(NetworkAddress),
    Refresh(NetworkAddress),
}

#[derive(Clone, Debug)]
pub struct NetworkListing {
    pub address: NetworkAddress,
    /// The ancestors of `address` that its provider reports, the outermost first and
    /// the parent last; empty at a root. The parent is present whenever the provider
    /// has one. Further out the chain may end early, when the provider stops
    /// answering or the engine's bounds are reached, so the first one need not be a
    /// root. Each is a `Browse` target; no caption is ever parsed into an address.
    pub ancestors: Arc<[NetworkAddress]>,
    pub entries: Arc<[crate::Entry]>,
    pub index: Arc<crate::DirectoryIndex>,
}
impl NetworkListing {
    /// The provider's parent of `address`, which Up opens.
    pub fn parent(&self) -> Option<&NetworkAddress> {
        self.ancestors.last()
    }
}

#[derive(Clone, Debug)]
pub enum NetworkUpdate {
    Listing {
        listing: NetworkListing,
        done: bool,
        error: Option<NetworkFailure>,
    },
    Native(PathBuf),
    /// Explicit activation paused without changing the committed location.
    SignInRequired {
        address: NetworkAddress,
        rejected: bool,
    },
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
    fn supports_sign_in(&self) -> bool {
        false
    }
    fn sign_in(
        &self,
        _address: &NetworkAddress,
        _credentials: &NetworkCredentials,
        _cancel: &Cancellation,
    ) -> Result<NetworkDestination, NetworkFailure> {
        Err(NetworkFailure::new(
            NetworkFailureKind::Unsupported,
            "This network provider does not support sign-in.",
        ))
    }
    fn refresh(
        &self,
        address: &NetworkAddress,
        cancel: &Cancellation,
    ) -> Result<(), NetworkFailure>;
}
