//! Bounded modal previews of already-captured diagnostic reports.

pub const DIAGNOSTIC_PREVIEW_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticTextPreview {
    pub text: String,
    pub total_bytes: u64,
    /// The original capture remains available for the existing full-report copy action.
    pub truncated: bool,
}
