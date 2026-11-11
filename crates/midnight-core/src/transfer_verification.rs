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
    /// Bytes independently read from one file under this immutable policy.
    /// This is a work estimate, not a completed-verification receipt.
    pub fn readback_bytes(self, length: u64) -> u64 {
        match self {
            Self::FullSha256 => length,
            Self::SizeAndSamples => length.min(3 * TRANSFER_SAMPLE_BYTES as u64),
        }
    }
    pub fn from_checksum_enabled(enabled: bool) -> Self {
        if enabled {
            Self::FullSha256
        } else {
            Self::SizeAndSamples
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readback_budget_matches_disjoint_samples_and_full_small_files() {
        for (length, sampled) in [
            (0, 0),
            (65_536, 65_536),
            (196_607, 196_607),
            (196_608, 196_608),
            (196_609, 196_608),
            (u64::MAX, 196_608),
        ] {
            assert_eq!(
                TransferVerification::FullSha256.readback_bytes(length),
                length
            );
            assert_eq!(
                TransferVerification::SizeAndSamples.readback_bytes(length),
                sampled
            );
        }
    }
}
