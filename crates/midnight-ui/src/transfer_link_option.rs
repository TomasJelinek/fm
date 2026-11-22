//! The Copy/Move dialog's link choice, the same for Copy and Move on Windows and Linux:
//! copy the files links point to (`CopyLinkPolicy::FollowLinks`) or keep links as links
//! (`CopyLinkPolicy::Preserve`). Display and request policy only; the native providers
//! decide what each policy does, and the presenter chooses the default for the
//! destination (`presenter::copy_dialog_links`).
use midnight_core::CopyLinkPolicy;

/// The choice's entries, in the order the dialog lists them: index 0 copies the files.
pub(crate) const COPY_FILES: &str = "Copy the files links point to";
pub(crate) const KEEP_LINKS: &str = "Keep links as links";

/// What each choice does, in plain words.
pub(crate) const HINT: &str = "Copy the files links point to: a link to a file becomes a copy of that file, a link to a folder a copy of the folder with everything in it, also from another drive. A Move then removes only the link itself, never the files it pointed to. A link that cannot be followed (its target is missing or unreadable, a loop of links, a device or a system folder such as /proc) stays a link, and the job details say why.\nKeep links as links: each link is copied or moved as a link with exactly the same target, and what it points to is not copied. Where links cannot be created, each one is left out and named in the job details.";

/// The choice's tooltip: why the destination's default was chosen, when it has a reason
/// (`reason`, empty where keeping links is the default), then what each choice does.
pub(crate) fn hint(reason: &str) -> String {
    if reason.is_empty() {
        HINT.to_owned()
    } else {
        format!("{reason}\n\n{HINT}")
    }
}

/// The policy for the dialog's choice; `copy_files` is None when the choice is hidden.
pub(crate) fn policy(copy_files: Option<bool>) -> Option<CopyLinkPolicy> {
    copy_files.map(|copy_files| {
        if copy_files {
            CopyLinkPolicy::FollowLinks
        } else {
            CopyLinkPolicy::Preserve
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_and_move_copy_the_files_or_keep_links_alike() {
        assert_eq!(policy(Some(true)), Some(CopyLinkPolicy::FollowLinks));
        assert_eq!(policy(Some(false)), Some(CopyLinkPolicy::Preserve));
        assert_eq!(policy(None), None);
        assert_eq!(COPY_FILES, "Copy the files links point to");
        assert_eq!(KEEP_LINKS, "Keep links as links");
    }

    #[test]
    fn the_hint_explains_both_choices_and_that_a_move_never_removes_the_targets() {
        let (copy, keep) = HINT
            .split_once("\nKeep links as links: ")
            .expect("the hint explains keeping links");
        assert!(copy.starts_with("Copy the files links point to: "));
        assert!(copy.contains("removes only the link itself, never the files it pointed to"));
        assert!(copy.contains("stays a link, and the job details say why"));
        assert!(keep.starts_with("each link is copied or moved as a link with exactly the same"));
        assert!(keep.ends_with("named in the job details."));
    }

    #[test]
    fn the_tooltip_starts_with_why_the_default_was_chosen() {
        assert_eq!(hint(""), HINT);
        let reason = "Removable drive: the files links point to are copied, so that no link \
                      there points back to this computer.";
        assert_eq!(hint(reason), format!("{reason}\n\n{HINT}"));
    }
}
