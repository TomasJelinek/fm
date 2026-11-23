//! The synthetic document folder, and assets below it.
use super::*;

/// The URL Chromium makes of `reference` written in a document at `base`: RFC 3986
/// reference resolution, whose `..` segments stop at the origin's root. Only path
/// references, with an optional query and fragment, are needed by these tests.
pub(crate) fn resolved(base: &str, reference: &str) -> String {
    let (scheme, rest) = base.split_once("://").expect("an absolute base URL");
    let (authority, base_path) = rest.split_once('/').expect("a base path");
    let end = reference.find(['?', '#']).unwrap_or(reference.len());
    let (path, suffix) = reference.split_at(end);
    let merged = match path.strip_prefix('/') {
        Some(absolute) => absolute.to_owned(),
        None => {
            let folder = base_path.rfind('/').map_or("", |at| &base_path[..=at]);
            format!("{folder}{path}")
        }
    };
    let mut segments = Vec::new();
    let mut folder = false;
    for segment in merged.split('/') {
        folder = matches!(segment, "." | "..");
        match segment {
            "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    let mut path = segments.join("/");
    if folder && !path.is_empty() {
        path.push('/');
    }
    format!("{scheme}://{authority}/{path}{suffix}")
}

fn offline() -> DocumentPolicy {
    DocumentPolicy::new(1, false, false).unwrap()
}

#[test]
fn the_test_resolver_clamps_parent_folders_at_the_origin_root_as_chromium_does() {
    let base = "https://o.invalid/a/b/";
    assert_eq!(resolved(base, "c.html"), "https://o.invalid/a/b/c.html");
    assert_eq!(resolved(base, "../c.html"), "https://o.invalid/a/c.html");
    assert_eq!(resolved(base, "../../c.html"), "https://o.invalid/c.html");
    assert_eq!(
        resolved(base, "../../../c.html"),
        "https://o.invalid/c.html"
    );
    assert_eq!(resolved(base, "./x/../?q#f"), "https://o.invalid/a/b/?q#f");
    assert_eq!(resolved(base, ""), base);
    assert_eq!(resolved(base, "/c"), "https://o.invalid/c");
}

#[test]
fn the_document_is_served_sixteen_distinct_synthetic_folders_below_mewv_document() {
    let policy = offline();
    let main = policy.main_url();
    let folders: Vec<_> = main
        .strip_prefix("https://mewv-1.invalid/")
        .and_then(|path| path.strip_suffix('/'))
        .expect("the document is served at its folder's URL")
        .split('/')
        .collect();
    assert_eq!(SYNTHETIC_LEVELS, 16);
    assert_eq!(folders.len(), 1 + SYNTHETIC_LEVELS);
    assert_eq!(folders[0], "mewv-document");
    assert_eq!(folders, DocumentPolicy::document_folders());
    let distinct: std::collections::HashSet<_> = folders.iter().collect();
    assert_eq!(distinct.len(), folders.len());
    for folder in &folders {
        assert!(
            !folder.is_empty()
                && folder
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
            "{folder}"
        );
    }
    assert_eq!(main, DocumentPolicy::main_url_of(1));
    // State keeps reporting the directory it reported before the synthetic levels.
    assert_eq!(
        policy.display_url(),
        "https://mewv-1.invalid/mewv-document/"
    );
}

#[test]
fn assets_keep_their_relative_urls_and_refusals_below_the_synthetic_folders() {
    let policy = offline();
    let main = policy.main_url();
    let seventeen = format!("{}x.css", "../".repeat(17));
    for (reference, access, relative) in [
        ("site.css", Access::Captured, Some("site.css")),
        (
            "css/site.css?v=2",
            Access::Captured,
            Some("css/site.css?v=2"),
        ),
        ("img/a%20b.png", Access::Captured, Some("img/a%20b.png")),
        (
            "css/../img/logo.png",
            Access::Captured,
            Some("img/logo.png"),
        ),
        ("./fonts/./a.woff2", Access::Captured, Some("fonts/a.woff2")),
        // The document itself is no asset.
        ("", Access::Captured, None),
        // Assets of folders above the document's go to the host as `../` references
        // (rustdoc's `../static.files/`, MkDocs' `../css/`); the host decides how far up
        // it reaches.
        ("../site.css", Access::Captured, Some("../site.css")),
        (
            "../../css/site.css?v=1#x",
            Access::Captured,
            Some("../../css/site.css?v=1"),
        ),
        (
            "../../static.files/main%20page.js",
            Access::Captured,
            Some("../../static.files/main%20page.js"),
        ),
        // A folder names no asset; nothing above the 16 levels or root-relative leaves.
        ("../", Access::Deny, None),
        (seventeen.as_str(), Access::Deny, None),
        ("/site.css", Access::Deny, None),
    ] {
        let url = resolved(&main, reference);
        assert_eq!(policy.access(&url), access, "{reference}");
        assert_eq!(
            policy.captured_path(&url).as_deref(),
            relative,
            "{reference}"
        );
    }
    let sixteen = format!("{}x.css", "../".repeat(16));
    assert_eq!(
        policy.captured_path(&resolved(&main, &sixteen)).as_deref(),
        Some(sixteen.as_str())
    );
    let online = DocumentPolicy::new(1, true, false).unwrap();
    for reference in ["css/site.css", "../site.css"] {
        assert_eq!(
            online.access(&resolved(&main, reference)),
            Access::Deny,
            "{reference}"
        );
    }
    assert_eq!(online.access(&resolved(&main, "?q")), Access::Captured);
}

/// Web viewer API 0.4: offline scripts run in a sandboxed document with an opaque origin
/// that reaches no network and reads no captured file.
#[test]
fn offline_scripts_run_sandboxed_without_any_connection() {
    let policy = DocumentPolicy::new(3, false, true).unwrap();
    assert!(policy.sandboxed());
    let csp = policy.csp();
    let directives: Vec<_> = csp.split("; ").collect();
    assert_eq!(directives[0], "sandbox allow-scripts allow-popups");
    assert!(!csp.contains("allow-same-origin") && !csp.contains("'self'"));
    for directive in [
        "connect-src 'none'",
        "object-src 'none'",
        "frame-src 'none'",
        "child-src 'none'",
        "worker-src 'none'",
        "form-action 'none'",
        "default-src https://mewv-3.invalid data: blob:",
        "base-uri https://mewv-3.invalid",
    ] {
        assert!(directives.contains(&directive), "{directive} in {csp}");
    }
    assert!(!csp.contains("http:") && !csp.contains("https: "));
    // Offline without scripts and online stay as they were.
    assert!(!offline().sandboxed());
    assert!(offline().csp().contains("script-src 'none'"));
    for scripts in [false, true] {
        let online = DocumentPolicy::new(3, true, scripts).unwrap();
        assert!(!online.sandboxed());
        assert!(!online.csp().contains("sandbox"));
    }
    assert!(DocumentPolicy::new(0, false, true).is_err());
}

#[test]
fn captured_asset_urls_keep_their_budget() {
    let policy = offline();
    let main = policy.main_url();
    let longest = "a".repeat(MAX_RESOURCE_URL_BYTES);
    assert_eq!(MAX_RESOURCE_URL_BYTES, 8192);
    assert_eq!(
        policy.captured_path(&format!("{main}{longest}")),
        Some(longest.clone())
    );
    assert_eq!(policy.captured_path(&format!("{main}{longest}a")), None);
    assert_eq!(policy.captured_path(&main), None);
}
