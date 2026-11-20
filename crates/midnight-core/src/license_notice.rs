//! The About dialog's library details. A request names one notice folder; the
//! backend answers with bounded text and never with paths.

/// Identifies one detail request; the UI ignores answers to older requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LicenseNoticeId(pub u64);

/// One library's notices. A Cargo package is found by name and version in the
/// release package's `licenses` folder (debug builds also look in Cargo's
/// registry); a bundled component names its own folder there instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LicenseNoticeRequest {
    pub id: LicenseNoticeId,
    pub name: String,
    pub version: String,
    /// `None` for a Cargo package (`<name>-<version>`), otherwise a bundled
    /// component's folder relative to `licenses`, such as `rust` or `themes/folke`.
    pub folder: Option<String>,
}

/// The bounded answer. `text` is empty when no notice folder was found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LicenseNotice {
    pub id: LicenseNoticeId,
    /// Manifest authors when the package records them, otherwise the copyright
    /// lines of its notices.
    pub authors: Vec<String>,
    pub text: String,
    /// Notice text was left out to stay within the display bound.
    pub truncated: bool,
}
