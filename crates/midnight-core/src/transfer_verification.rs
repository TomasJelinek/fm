//! Immutable integrity policy captured when a transfer is submitted.

/// Bytes compared at the beginning, middle and end in opt-in quick verification.
pub const TRANSFER_SAMPLE_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransferVerification {
    /// Independently read and SHA-256 hash the entire destination. The default.
    #[default]
    FullSha256,
    /// Check length and the first/middle/last64 KiB. Files up to192 KiB are
    /// compared entirely. Corruption between samples can go undetected.
    SizeAndSamples,
}

impl TransferVerification {
    pub fn from_checksum_enabled(enabled: bool) -> Self {
        if enabled {
            Self::FullSha256
        } else {
            Self::SizeAndSamples
        }
    }
}
