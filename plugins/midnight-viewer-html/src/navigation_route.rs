//! CEF-free choice between original navigation and captured-body preparation.
use crate::policy::{Access, DocumentPolicy};

#[cfg(test)]
#[path = "navigation_route_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowseRoute {
    Refuse,
    Original,
    Bootstrap,
    Prepare,
}

pub(crate) struct BrowseFacts<'a> {
    /// Actual marked reissues are validated before this unmarked route selector.
    pub unmarked: bool,
    pub target: &'a str,
    pub main_frame: bool,
    /// Supplied only after the callback frame matches the browser's valid current
    /// main frame. This is its real committed URL, never provider State.url.
    pub current_main_url: Option<&'a str>,
    pub get_no_post: bool,
    pub bootstrap_unused: bool,
    pub same_document_fragment: bool,
    pub redirect: bool,
    pub direct_load: bool,
}

pub(crate) fn classify(policy: &DocumentPolicy, facts: &BrowseFacts<'_>) -> BrowseRoute {
    if !facts.unmarked {
        return BrowseRoute::Refuse;
    }
    let access = policy.access(facts.target);
    if access == Access::Deny {
        return BrowseRoute::Refuse;
    }
    if !facts.main_frame || access != Access::Captured || !facts.get_no_post {
        return BrowseRoute::Original;
    }
    let main_url = policy.main_url();
    let captured_root = facts.target.split(['?', '#']).next() == Some(main_url.as_str());
    // Keep this ORIGINAL request, including its redirects/direct-load flags.
    // It must not become a canceled request followed by unsafe cross-origin
    // LoadRequest, and online policy alone is not the exception criterion.
    if captured_root
        && facts
            .current_main_url
            .is_some_and(|url| policy.access(url) == Access::Network)
    {
        return BrowseRoute::Original;
    }
    if facts.bootstrap_unused && facts.target == main_url {
        return BrowseRoute::Bootstrap;
    }
    if facts.direct_load {
        return BrowseRoute::Refuse;
    }
    let private_origin = format!("{}/", policy.origin());
    let current_private = facts
        .current_main_url
        .is_some_and(|url| url.starts_with(&private_origin));
    if current_private && facts.same_document_fragment {
        return BrowseRoute::Original;
    }
    if facts.redirect || !current_private {
        return BrowseRoute::Refuse;
    }
    BrowseRoute::Prepare
}
