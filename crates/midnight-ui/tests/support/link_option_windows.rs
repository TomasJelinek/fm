//! Windows keeps the directory-link options for Copy and Move.
use midnight_core::CopyLinkPolicy;

pub const LABEL: &str = "Follow directory links";

pub fn shown(_moving: bool) -> bool {
    true
}

pub fn sent(_moving: bool, follow: bool) -> CopyLinkPolicy {
    if follow {
        CopyLinkPolicy::FollowDirectoryContents
    } else {
        CopyLinkPolicy::EmptyDirectories
    }
}
