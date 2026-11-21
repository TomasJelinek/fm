//! Linux: Copy follows links or keeps them as links; Move always moves a link itself.
use midnight_core::{CopyLinkPolicy, TransferIntent};

pub(crate) const LABEL: &str = "Follow links";
/// Both states, as `CopyLinkPolicy::FollowLinks` and `Preserve` behave on Linux
/// (transfer_directory_links.rs in midnight-platform-linux).
pub(crate) const HINT: &str = "Checked: copy what each link points to, a file's content or a folder with everything in it, also on another filesystem, where that is possible. A link that cannot be followed (its target is missing or unreadable, a device, pipe or socket, a loop of links, on a system filesystem such as /proc, or a folder that contains the link) is copied as a link, and the job details say why.\nUnchecked: copy each link itself, as a link with exactly the same target text, without copying what it points to.\nMove always moves links as links.";

/// Move moves links themselves, so only Copy offers the option.
pub(crate) fn visible(intent: TransferIntent) -> bool {
    intent == TransferIntent::Copy
}

/// The policy for the dialog's choice; `follow` is None when the option is hidden.
pub(crate) fn policy(intent: TransferIntent, follow: Option<bool>) -> Option<CopyLinkPolicy> {
    match intent {
        TransferIntent::Move => Some(CopyLinkPolicy::Preserve),
        TransferIntent::Copy => follow.map(|follow| {
            if follow {
                CopyLinkPolicy::FollowLinks
            } else {
                CopyLinkPolicy::Preserve
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_follows_or_keeps_links_and_move_always_moves_the_link() {
        assert_eq!(
            policy(TransferIntent::Copy, Some(true)),
            Some(CopyLinkPolicy::FollowLinks)
        );
        assert_eq!(
            policy(TransferIntent::Copy, Some(false)),
            Some(CopyLinkPolicy::Preserve)
        );
        assert_eq!(policy(TransferIntent::Copy, None), None);
        for follow in [Some(true), Some(false), None] {
            assert_eq!(
                policy(TransferIntent::Move, follow),
                Some(CopyLinkPolicy::Preserve)
            );
        }
        assert!(visible(TransferIntent::Copy) && !visible(TransferIntent::Move));
    }

    #[test]
    fn the_hint_explains_both_states_and_move() {
        let (checked, rest) = HINT
            .split_once("\nUnchecked: ")
            .expect("the hint explains the cleared state");
        assert!(checked.starts_with("Checked: copy what each link points to"));
        assert!(checked.contains("is copied as a link, and the job details say why"));
        assert!(
            rest.starts_with("copy each link itself, as a link with exactly the same target text")
        );
        assert!(rest.ends_with("\nMove always moves links as links."));
    }
}
