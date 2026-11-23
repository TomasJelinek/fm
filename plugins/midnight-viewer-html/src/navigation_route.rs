//! CEF-free choice between original navigation and captured-body preparation, or under
//! host link routing between the host and a refusal.
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
    /// Host link routing: cancel the navigation and hand its target to the host.
    Activate,
    /// Host link routing: cancel the navigation and tell the host why, with this notice.
    Deny(&'static str),
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
    /// Chromium's client-redirect transition flag: a navigation the page started itself,
    /// such as a `<meta http-equiv="refresh">`. Blink may give it the user's gesture after
    /// a recent click or key press, so the gesture alone does not make it a followed link.
    pub client_redirect: bool,
    /// Chromium's form-submit transition: a form sent with GET or POST.
    pub form_submit: bool,
    pub direct_load: bool,
    /// Chromium's user-gesture flag of the navigation.
    pub user_gesture: bool,
    /// The host launched the provider with host link routing (`links.rs`).
    pub links: bool,
}

pub(crate) fn classify(policy: &DocumentPolicy, facts: &BrowseFacts<'_>) -> BrowseRoute {
    if !facts.unmarked {
        return BrowseRoute::Refuse;
    }
    if facts.links && facts.main_frame {
        return route_link(policy, facts);
    }
    let access = policy.access(facts.target);
    // Assets may come from folders above the document's (`DocumentPolicy::captured_path`),
    // but the browser shows no document of them by itself: only the host opens those.
    if access == Access::Deny
        || (facts.main_frame
            && access == Access::Captured
            && !facts.target.starts_with(&policy.main_url()))
    {
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

/// Host link routing: the main frame never shows another document by itself, wherever
/// the navigation leads (the host decides). The browser's first load of the document and
/// fragment navigations within it stay inside the browser; a navigation the user started
/// goes to the host; anything else is refused with a notice.
fn route_link(policy: &DocumentPolicy, facts: &BrowseFacts<'_>) -> BrowseRoute {
    if facts.bootstrap_unused && facts.get_no_post && facts.target == policy.main_url() {
        return BrowseRoute::Bootstrap;
    }
    let private_origin = format!("{}/", policy.origin());
    let current_private = facts
        .current_main_url
        .is_some_and(|url| url.starts_with(&private_origin));
    if current_private && facts.same_document_fragment && !facts.direct_load {
        return BrowseRoute::Original;
    }
    // Forms are not submitted, by the user (GET or POST) or by the page itself.
    if facts.form_submit {
        return BrowseRoute::Deny(crate::links::FORM);
    }
    if !facts.user_gesture || facts.redirect || facts.client_redirect || facts.direct_load {
        return BrowseRoute::Deny(crate::links::BY_ITSELF);
    }
    if !facts.get_no_post {
        return BrowseRoute::Deny(crate::links::FORM);
    }
    BrowseRoute::Activate
}
