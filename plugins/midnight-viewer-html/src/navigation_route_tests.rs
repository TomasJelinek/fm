use super::*;
use crate::links::{BY_ITSELF, FORM};
use crate::policy::tests::resolved;
use std::sync::LazyLock;

/// The generation-1 document URL ([`DocumentPolicy::main_url`]), below its origin's root.
static MAIN: LazyLock<String> = LazyLock::new(|| DocumentPolicy::main_url_of(1));

fn main_url() -> &'static str {
    MAIN.as_str()
}

fn online() -> DocumentPolicy {
    DocumentPolicy::new(1, true, false).unwrap()
}

fn offline() -> DocumentPolicy {
    DocumentPolicy::new(1, false, false).unwrap()
}

fn network_return() -> BrowseFacts<'static> {
    BrowseFacts {
        unmarked: true,
        target: main_url(),
        main_frame: true,
        current_main_url: Some("http://127.0.0.1:32123/roundtrip"),
        get_no_post: true,
        bootstrap_unused: false,
        same_document_fragment: false,
        redirect: false,
        direct_load: false,
        user_gesture: false,
        links: false,
    }
}

#[test]
fn validated_network_root_return_keeps_original_redirect_and_direct_load_flags() {
    assert_eq!(online().main_url(), main_url());
    for (redirect, direct_load) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut facts = network_return();
        facts.redirect = redirect;
        facts.direct_load = direct_load;
        assert_eq!(classify(&online(), &facts), BrowseRoute::Original);
    }
    let mut facts = network_return();
    let query = format!("{}?query#fragment", main_url());
    facts.target = &query;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Original);
}

#[test]
fn private_origin_still_prepares_under_online_policy_including_scripts() {
    let mut facts = network_return();
    facts.current_main_url = Some(main_url());
    for scripts in [false, true] {
        let policy = DocumentPolicy::new(1, true, scripts).unwrap();
        assert_eq!(classify(&policy, &facts), BrowseRoute::Prepare);
    }
    let page = format!("{}page.html", main_url());
    facts.target = &page;
    assert_eq!(classify(&offline(), &facts), BrowseRoute::Prepare);
}

#[test]
fn missing_invalid_or_unknown_current_main_frame_has_no_network_return_exception() {
    for current in [
        None,
        Some("about:blank"),
        Some("file:///C:/unrelated.html"),
        Some("https://mewv-9.invalid/mewv-document/"),
        Some("https://mewv-1.invalid.evil/"),
    ] {
        let mut facts = network_return();
        // The bridge supplies None when actual browser/current-main identity
        // validation fails; a caller-supplied online policy cannot replace it.
        facts.current_main_url = current;
        assert_eq!(classify(&online(), &facts), BrowseRoute::Refuse);
    }
}

#[test]
fn marked_or_lost_marker_attempt_cannot_use_original_network_return() {
    let mut facts = network_return();
    facts.unmarked = false;
    facts.redirect = true;
    facts.direct_load = true;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Refuse);
    facts.current_main_url = Some(main_url());
    facts.unmarked = true;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Refuse);
}

#[test]
fn original_return_never_overrides_target_policy_or_old_generation_authority() {
    for target in [
        "https://mewv-1.invalid/",
        "https://mewv-1.invalid/asset.html",
        "https://mewv-1.invalid/mewv-document/page.html",
        "https://mewv-1.invalid/mewv-document-other/page.html",
        "https://mewv-2.invalid/",
        "file:///C:/source.html",
        "javascript:alert(1)",
    ] {
        let mut facts = network_return();
        facts.target = target;
        assert_eq!(classify(&online(), &facts), BrowseRoute::Refuse);
    }
    assert_eq!(classify(&offline(), &network_return()), BrowseRoute::Refuse);
}

#[test]
fn private_origin_keeps_strong_redirect_and_direct_load_refusals() {
    for (redirect, direct_load) in [(true, false), (false, true), (true, true)] {
        let mut facts = network_return();
        facts.current_main_url = Some(main_url());
        facts.redirect = redirect;
        facts.direct_load = direct_load;
        assert_eq!(classify(&online(), &facts), BrowseRoute::Refuse);
    }
}

#[test]
fn bootstrap_non_get_network_and_subframe_boundaries_remain_ordinary() {
    let mut facts = network_return();
    facts.current_main_url = None;
    facts.bootstrap_unused = true;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Bootstrap);
    facts.bootstrap_unused = false;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Refuse);
    facts.get_no_post = false;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Original);
    facts.get_no_post = true;
    facts.main_frame = false;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Original);
    facts.main_frame = true;
    facts.target = "https://example.org/ordinary-network";
    assert_eq!(classify(&online(), &facts), BrowseRoute::Original);
}

#[test]
fn same_document_fragment_preserves_original_path_after_strong_only_direct_check() {
    let mut facts = network_return();
    facts.current_main_url = Some(main_url());
    let part = format!("{}#part", main_url());
    facts.target = &part;
    facts.same_document_fragment = true;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Original);
    facts.direct_load = true;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Refuse);
}

/// A navigation of the main frame, which shows the captured document, under host link
/// routing: a link the user followed.
fn link(target: &str) -> BrowseFacts<'_> {
    BrowseFacts {
        unmarked: true,
        target,
        main_frame: true,
        current_main_url: Some(main_url()),
        get_no_post: true,
        bootstrap_unused: false,
        same_document_fragment: false,
        redirect: false,
        direct_load: false,
        user_gesture: true,
        links: true,
    }
}

#[test]
fn host_link_routing_hands_every_followed_link_to_the_host_wherever_it_leads() {
    let main = main_url();
    for target in [
        resolved(main, "sub/other.html#part"),
        resolved(main, "../README.md"),
        resolved(main, &format!("{}top.html", "../".repeat(17))),
        resolved(main, "office/v1/sheet/1/first.html"),
        resolved(main, ""),
        "https://example.org/".into(),
        "mailto:someone@example.org".into(),
        "file:///C:/elsewhere.html".into(),
    ] {
        for policy in [offline(), online()] {
            assert_eq!(
                classify(&policy, &link(&target)),
                BrowseRoute::Activate,
                "{target}"
            );
        }
    }
}

#[test]
fn host_link_routing_refuses_what_the_page_starts_by_itself_with_a_notice() {
    let target = resolved(main_url(), "next.html");
    let mut facts = link(&target);
    facts.user_gesture = false;
    assert_eq!(classify(&offline(), &facts), BrowseRoute::Deny(BY_ITSELF));
    for (redirect, direct_load) in [(true, false), (false, true)] {
        let mut facts = link(&target);
        facts.redirect = redirect;
        facts.direct_load = direct_load;
        assert_eq!(classify(&offline(), &facts), BrowseRoute::Deny(BY_ITSELF));
    }
    let mut facts = link(&target);
    facts.get_no_post = false;
    assert_eq!(classify(&offline(), &facts), BrowseRoute::Deny(FORM));
    // Without the user's gesture, a link elsewhere is refused all the same.
    let mut facts = link("https://example.org/");
    facts.user_gesture = false;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Deny(BY_ITSELF));
}

#[test]
fn same_document_fragments_stay_in_the_browser_under_host_link_routing() {
    let part = format!("{}#part", main_url());
    let old = format!("{}#old", main_url());
    for (current, gesture) in [(main_url(), true), (main_url(), false), (&*old, true)] {
        let mut facts = link(&part);
        facts.current_main_url = Some(current);
        facts.same_document_fragment = true;
        facts.user_gesture = gesture;
        assert_eq!(classify(&offline(), &facts), BrowseRoute::Original);
    }
}

#[test]
fn the_first_load_stays_in_the_browser_under_host_link_routing() {
    let mut facts = link(main_url());
    facts.current_main_url = None;
    facts.bootstrap_unused = true;
    facts.user_gesture = false;
    facts.direct_load = true;
    assert_eq!(classify(&offline(), &facts), BrowseRoute::Bootstrap);
    facts.bootstrap_unused = false;
    assert_eq!(classify(&offline(), &facts), BrowseRoute::Deny(BY_ITSELF));
}

#[test]
fn subframes_keep_their_routes_under_host_link_routing() {
    let asset = resolved(main_url(), "frame.html");
    for links in [false, true] {
        let mut facts = link(&asset);
        facts.main_frame = false;
        facts.links = links;
        assert_eq!(classify(&offline(), &facts), BrowseRoute::Original);
        let mut facts = link("https://example.org/");
        facts.main_frame = false;
        facts.links = links;
        assert_eq!(classify(&offline(), &facts), BrowseRoute::Refuse);
    }
}

#[test]
fn without_the_opt_in_links_keep_loading_in_the_browser_including_office_paging() {
    let main = main_url();
    let catalog = resolved(main, "office/v1/catalog.html");
    let sheet = resolved(main, "office/v1/sheet/0/first.html");
    for gesture in [false, true] {
        for (current, reference) in [
            (main, "office/v1/catalog.html"),
            (&*catalog, "../../office/v1/sheet/1/first.html"),
            (&*sheet, "../../../../office/v1/sheet/1/first.html"),
            (&*sheet, "../../../../office/v1/sheet/0/row/129/col/1.html"),
            (main, "sub/page.html?q=1"),
        ] {
            let target = resolved(current, reference);
            let mut facts = link(&target);
            facts.current_main_url = Some(current);
            facts.links = false;
            facts.user_gesture = gesture;
            assert_eq!(
                classify(&offline(), &facts),
                BrowseRoute::Prepare,
                "{reference}"
            );
        }
        for target in [
            resolved(main, "../above.html"),
            "https://example.org/".into(),
        ] {
            let mut facts = link(&target);
            facts.links = false;
            facts.user_gesture = gesture;
            assert_eq!(classify(&offline(), &facts), BrowseRoute::Refuse);
        }
    }
}
