use crate::ProviderId;
use std::{fmt, num::NonZeroU32, sync::Arc};

/// Persistent provider-owned address, kept verbatim even when its plugin is absent.
///
/// This is neither a native path nor a URL. The provider's versioned codec alone
/// interprets it. Callers bound aggregate storage/IPC allocation rather than
/// imposing a universal path-length limit. Addresses must not contain credentials.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct StoredLocation {
    provider: ProviderId,
    codec_version: NonZeroU32,
    address: Arc<[u8]>,
}

impl StoredLocation {
    pub fn new(provider: ProviderId, codec_version: NonZeroU32, address: Arc<[u8]>) -> Self {
        Self {
            provider,
            codec_version,
            address,
        }
    }

    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    pub fn codec_version(&self) -> NonZeroU32 {
        self.codec_version
    }

    pub fn address(&self) -> &[u8] {
        &self.address
    }
}

impl fmt::Debug for StoredLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredLocation")
            .field("provider", &self.provider)
            .field("codec_version", &self.codec_version)
            .field("address_bytes", &self.address.len())
            .finish()
    }
}
