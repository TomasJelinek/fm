//! Linux: Copy follows links or keeps them as links; Move always moves a link itself.
use midnight_core::CopyLinkPolicy;

pub const LABEL: &str = "Follow links";

pub fn shown(moving: bool) -> bool {
    !moving
}

pub fn sent(moving: bool, follow: bool) -> CopyLinkPolicy {
    match (moving, follow) {
        (true, _) => CopyLinkPolicy::Preserve,
        (false, true) => CopyLinkPolicy::FollowLinks,
        (false, false) => CopyLinkPolicy::Preserve,
    }
}
