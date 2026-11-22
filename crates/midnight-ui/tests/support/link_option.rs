//! The Copy/Move dialog's link choice, the same on Windows and Linux and for Copy and
//! Move: its accessible label, its two entries and the policy each sends. Expected
//! values only.
use midnight_core::CopyLinkPolicy;

/// The accessible label of the choice (a combo box).
pub const LABEL: &str = "Links";
pub const COPY_FILES: &str = "Copy the files links point to";
pub const KEEP_LINKS: &str = "Keep links as links";

/// Copy and Move both show the choice.
pub fn shown(_moving: bool) -> bool {
    true
}

/// The policy the dialog sends for its choice.
pub fn sent(_moving: bool, copy_files: bool) -> CopyLinkPolicy {
    if copy_files {
        CopyLinkPolicy::FollowLinks
    } else {
        CopyLinkPolicy::Preserve
    }
}

/// Whether `command` is the dialog's question about its destination's links, which it
/// asks when it opens.
pub fn inspection(command: &midnight_core::Command) -> bool {
    matches!(command, midnight_core::Command::InspectTransferLinks(_))
}
