//! Host routing identities. These are not plugin ABI handles or mutation authority.
use std::{fmt, num::NonZeroU64, sync::Arc};

/// Stable package identity, independent of labels and native path spelling.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProviderId(Arc<str>);

impl ProviderId {
    pub fn new(value: &str) -> Result<Self, IdentityError> {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(IdentityError("Invalid filesystem provider identity."));
        }
        Ok(Self(Arc::from(value)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One registration lifetime. The host allocates it without wrapping or reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProviderInstanceId(pub NonZeroU64);

/// One live mount lifetime; never persisted or derived from displayed text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MountRef {
    pub instance: ProviderInstanceId,
    pub mount: NonZeroU64,
    pub generation: NonZeroU64,
}

/// Small provider-owned token. Long paths belong in provider storage, not tokens.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct NodeToken(Arc<[u8]>);

impl NodeToken {
    pub const MAX_BYTES: usize = 256;

    pub fn new(bytes: &[u8]) -> Result<Self, IdentityError> {
        if bytes.is_empty() || bytes.len() > Self::MAX_BYTES {
            return Err(IdentityError("Invalid filesystem node token length."));
        }
        Ok(Self(Arc::from(bytes)))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for NodeToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeToken")
            .field("bytes", &self.0.len())
            .finish()
    }
}

/// A node token in exactly one mount, not an inode or cross-provider object ID.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeRef {
    pub mount: MountRef,
    pub token: NodeToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityError(pub &'static str);

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for IdentityError {}
