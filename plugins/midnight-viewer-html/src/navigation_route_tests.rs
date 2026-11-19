use super::*;

/// The generation-1 document URL ([`DocumentPolicy::main_url`]), below its origin's root.
const MAIN: &str = "https://mewv-1.invalid/mewv-document/";

fn online() -> DocumentPolicy {
    DocumentPolicy::new(1, true, false).unwrap()
}

fn network_return() -> BrowseFacts<'static> {
    BrowseFacts {
        unmarked: true,
        target: MAIN,
        main_frame: true,
        current_main_url: Some("http://127.0.0.1:32123/roundtrip"),
        get_no_post: true,
        bootstrap_unused: false,
        same_document_fragment: false,
        redirect: false,
        direct_load: false,
    }
}

#[test]
fn validated_network_root_return_keeps_original_redirect_and_direct_load_flags() {
    assert_eq!(online().main_url(), MAIN);
    for (redirect, direct_load) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut facts = network_return();
        facts.redirect = redirect;
        facts.direct_load = direct_load;
        assert_eq!(classify(&online(), &facts), BrowseRoute::Original);
    }
    let mut facts = network_return();
    facts.target = "https://mewv-1.invalid/mewv-document/?query#fragment";
    assert_eq!(classify(&online(), &facts), BrowseRoute::Original);
}

#[test]
fn private_origin_still_prepares_under_online_policy_including_scripts() {
    let mut facts = network_return();
    facts.current_main_url = Some(MAIN);
    for scripts in [false, true] {
        let policy = DocumentPolicy::new(1, true, scripts).unwrap();
        assert_eq!(classify(&policy, &facts), BrowseRoute::Prepare);
    }
    let offline = DocumentPolicy::new(1, false, false).unwrap();
    facts.target = "https://mewv-1.invalid/mewv-document/page.html";
    assert_eq!(classify(&offline, &facts), BrowseRoute::Prepare);
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
    facts.current_main_url = Some(MAIN);
    facts.unmarked = true;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Refuse);
}

#[test]
fn original_return_never_overrides_target_policy_or_old_generation_authority() {
    for target in [
        "https://mewv-1.invalid/",
        "https://mewv-1.invalid/asset.html",
        "https://mewv-1.invalid/mewv-document-other/page.html",
        "https://mewv-2.invalid/",
        "file:///C:/source.html",
        "javascript:alert(1)",
    ] {
        let mut facts = network_return();
        facts.target = target;
        assert_eq!(classify(&online(), &facts), BrowseRoute::Refuse);
    }
    assert_eq!(
        classify(
            &DocumentPolicy::new(1, false, false).unwrap(),
            &network_return()
        ),
        BrowseRoute::Refuse
    );
}

#[test]
fn private_origin_keeps_strong_redirect_and_direct_load_refusals() {
    for (redirect, direct_load) in [(true, false), (false, true), (true, true)] {
        let mut facts = network_return();
        facts.current_main_url = Some(MAIN);
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
    facts.current_main_url = Some(MAIN);
    facts.target = "https://mewv-1.invalid/mewv-document/#part";
    facts.same_document_fragment = true;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Original);
    facts.direct_load = true;
    assert_eq!(classify(&online(), &facts), BrowseRoute::Refuse);
}
