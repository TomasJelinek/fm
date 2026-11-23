use midnight_viewer_html::policy::{Access, DocumentPolicy};
use midnight_viewer_html::surface::Layers;

#[test]
fn offline_origin_never_falls_through_to_network() {
    let policy = DocumentPolicy::new(1, false, false).unwrap();
    let main = policy.main_url();
    assert!(main.starts_with("https://mewv-1.invalid/"));
    assert_eq!(policy.access(&main), Access::Captured);
    assert_eq!(
        policy.access(&format!("{main}assets/index.css?v=2")),
        Access::Captured
    );
    assert_eq!(policy.access("https://example.com/"), Access::Deny);
    for url in [
        "file:///C:/secret",
        "javascript:alert(1)",
        "data:text/html,x",
        "https://mewv-1.invalid.evil/a",
        "https://mewv-1.invalid@evil/a",
        // "../site.css" from the document resolves above its root; denied rather
        // than clamped onto a same-named file of the document directory.
        "https://mewv-1.invalid/site.css",
        "https://mewv-1.invalid/",
        "https://mewv-2.invalid/mewv-document/a.css",
    ] {
        assert_eq!(policy.access(url), Access::Deny, "{url}");
    }
}

#[test]
fn online_only_exposes_main_document_and_offline_scripts_are_sandboxed() {
    // Web viewer API 0.4: scripts may run offline, in a sandboxed opaque-origin document.
    let offline = DocumentPolicy::new(1, false, true).unwrap();
    assert!(
        offline.sandboxed()
            && offline
                .csp()
                .starts_with("sandbox allow-scripts allow-popups;")
    );
    assert_eq!(offline.access("https://example.com/asset"), Access::Deny);
    let policy = DocumentPolicy::new(2, true, true).unwrap();
    let main = policy.main_url();
    assert_eq!(policy.access(&main), Access::Captured);
    assert_eq!(policy.access(&format!("{main}?query")), Access::Captured);
    assert_eq!(policy.access(&format!("{main}index.html")), Access::Deny);
    assert_eq!(policy.access(&format!("{main}sibling.css")), Access::Deny);
    assert_eq!(policy.access("https://example.com/asset"), Access::Network);
    assert_eq!(policy.access("file:///tmp/a"), Access::Deny);
}

#[test]
fn online_network_branch_rejects_private_userinfo_and_empty_authorities() {
    let policy = DocumentPolicy::new(2, true, false).unwrap();
    for url in [
        // A previous generation's private origin never becomes a network request.
        "https://mewv-1.invalid/",
        "https://mewv-1.invalid/mewv-document/",
        "https://example.invalid/",
        "https://user@example.com/",
        "https:///x",
        "http://",
    ] {
        assert_eq!(policy.access(url), Access::Deny, "{url}");
    }
    assert_eq!(policy.access("https://example.com/"), Access::Network);
    assert_eq!(policy.access("http://example.com/a?b"), Access::Network);
}

#[test]
fn zero_generation_reports_its_own_error() {
    assert_eq!(
        DocumentPolicy::new(0, false, false).unwrap_err(),
        "document generation must be nonzero"
    );
}

#[test]
fn view_paints_move_into_frames_and_popups_compose_on_a_fresh_base() {
    let base = vec![100, 100, 100, 255, 100, 100, 100, 255];
    let mut layers = Layers::default();
    // Without a popup the converted paint becomes the frame; no base copy is kept.
    assert_eq!(layers.view(2, 1, base.clone()), Some(base.clone()));
    assert!(layers.frame().is_none());
    layers.popup_rect(-1, 0);
    assert_eq!(layers.show(true), None);
    assert!(layers.popup(2, 1, &[0, 0, 0, 0, 0, 0, 128, 128]));
    // Composition waits for the fresh view paint requested on show.
    assert!(layers.frame().is_none());
    let composed = [178, 50, 50, 255, 100, 100, 100, 255];
    assert_eq!(layers.view(2, 1, base.clone()).unwrap(), composed);
    // Popup-only repaints recompose from the retained base.
    assert_eq!(layers.frame().unwrap(), composed);
    // Hiding returns the plain base and stops retaining it.
    assert_eq!(layers.show(false), Some(base));
    assert!(layers.frame().is_none());
    assert!(!layers.popup(8192, 8192, &[]));
    assert!(layers.view(2, 1, vec![0; 7]).is_none());
}
