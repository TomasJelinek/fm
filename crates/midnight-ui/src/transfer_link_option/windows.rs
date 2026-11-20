//! Windows keeps its directory-link options. Symbolic links (whose creation needs a
//! privilege) and junctions are not recreated as links yet; see TRANSFERS.md.
use midnight_core::{CopyLinkPolicy, TransferIntent};

pub(crate) const LABEL: &str = "Follow directory links";
pub(crate) const HINT: &str = "Follow qualified directory links and junctions when copying contents. Clear to create empty directories. Eligible native moves preserve the link itself.";

pub(crate) fn visible(_intent: TransferIntent) -> bool {
    true
}

/// The policy for the dialog's choice; `follow` is None when the option is hidden.
pub(crate) fn policy(_intent: TransferIntent, follow: Option<bool>) -> Option<CopyLinkPolicy> {
    follow.map(|follow| {
        if follow {
            CopyLinkPolicy::FollowDirectoryContents
        } else {
            CopyLinkPolicy::EmptyDirectories
        }
    })
}
