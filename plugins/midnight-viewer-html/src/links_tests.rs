use super::*;
use crate::policy::SYNTHETIC_LEVELS;
use crate::policy::tests::resolved;

fn policy() -> DocumentPolicy {
    DocumentPolicy::new(1, false, false).unwrap()
}

/// The target the provider reports for `reference` written in the captured document.
fn reported(reference: &str) -> Result<String, &'static str> {
    let policy = policy();
    target(&policy, &resolved(&policy.main_url(), reference))
}

fn hovered(target: &str) -> api::EventKind {
    api::EventKind::LinkHover {
        target: target.into(),
    }
}

fn activated(target: &str) -> api::EventKind {
    api::EventKind::LinkActivated {
        target: target.into(),
    }
}

fn denied(message: &str) -> api::EventKind {
    api::EventKind::Error {
        code: api::ErrorCode::Denied,
        message: message.into(),
    }
}

#[test]
fn only_the_exact_host_launch_value_routes_links_to_the_host() {
    assert!(enabled(Some("host-v1")));
    assert!(enabled(Some(api::LINKS_HOST_V1)));
    for value in [
        None,
        Some(""),
        Some("1"),
        Some("host-v2"),
        Some("host-v1 "),
        Some(" host-v1"),
        Some("HOST-V1"),
        Some("physical-v1"),
    ] {
        assert!(!enabled(value), "{value:?}");
    }
}

#[test]
fn private_origin_links_are_relative_to_the_documents_folder() {
    for (reference, expected) in [
        ("sub/other.html#part", "sub/other.html#part"),
        ("../README.md", "../README.md"),
        ("x.html?q=1", "x.html?q=1"),
        ("./sub/../x.html", "x.html"),
        ("../../a/b/", "../../a/b/"),
        ("a%20b.html", "a%20b.html"),
        ("a//b.html", "a//b.html"),
        (
            "office/v1/sheet/1/first.html",
            "office/v1/sheet/1/first.html",
        ),
        // The document is served at its folder's URL: both are "./".
        ("", "./"),
        (".", "./"),
        ("?page=2", "./?page=2"),
        ("#part", "./#part"),
        // A first segment with a colon would read as a scheme, and a leading slash as
        // a root path: "./" keeps both relative to the folder.
        ("./c:x.html", "./c:x.html"),
        (".//x.html", ".//x.html"),
    ] {
        assert_eq!(reported(reference).as_deref(), Ok(expected), "{reference}");
    }
}

#[test]
fn sixteen_parent_folders_are_told_apart_and_seventeen_are_refused() {
    assert_eq!(SYNTHETIC_LEVELS, 16);
    for parents in 1..=16 {
        let reference = format!("{}top.html", "../".repeat(parents));
        assert_eq!(reported(&reference), Ok(reference.clone()));
    }
    let sixteen = "../".repeat(16);
    assert_eq!(reported(&sixteen), Ok(sixteen.clone()), "a folder link");
    for parents in [17, 18, 40] {
        let reference = format!("{}top.html", "../".repeat(parents));
        assert_eq!(reported(&reference), Err(ABOVE), "{parents}");
    }
    assert_eq!(reported(&"../".repeat(17)), Err(ABOVE));
    // A root path names no place relative to the document's folder either.
    assert_eq!(reported("/top.html"), Err(ABOVE));
    assert_eq!(reported("/"), Err(ABOVE));
}

#[test]
fn a_real_folder_named_like_the_innermost_synthetic_level_aliases_the_documents_folder() {
    // Documented limit of the synthetic levels: their names are distinctive, not reserved.
    let innermost = DocumentPolicy::document_folders()[SYNTHETIC_LEVELS];
    assert_eq!(
        reported(&format!("../{innermost}/x.html")).as_deref(),
        Ok("x.html")
    );
}

#[test]
fn links_off_the_private_origin_are_passed_on_exactly_as_resolved() {
    let policy = policy();
    for url in [
        "https://example.org/a%20b?c=d#e",
        "http://127.0.0.1:8080/",
        "mailto:someone@example.org",
        "file:///C:/Users/x/a.html",
        "javascript:void(0)",
        "data:text/plain,hello",
        // Another document generation's private origin is no folder of this document.
        "https://mewv-2.invalid/mewv-document/a.html",
        "https://mewv-1.invalid.example/a",
    ] {
        assert_eq!(target(&policy, url).as_deref(), Ok(url));
    }
}

#[test]
fn targets_without_an_address_too_long_or_with_controls_are_refused() {
    let policy = policy();
    let main = policy.main_url();
    let longest = "x".repeat(api::MAX_LINK_TARGET_BYTES);
    assert_eq!(
        target(&policy, &format!("{main}{longest}")),
        Ok(longest.clone())
    );
    assert_eq!(target(&policy, &format!("{main}{longest}y")), Err(TOO_LONG));
    let external = format!(
        "https://e.org/{}",
        "x".repeat(api::MAX_LINK_TARGET_BYTES - 14)
    );
    assert_eq!(external.len(), api::MAX_LINK_TARGET_BYTES);
    assert_eq!(target(&policy, &external), Ok(external.clone()));
    assert_eq!(target(&policy, &format!("{external}y")), Err(TOO_LONG));
    let huge = format!("{main}{}", "x".repeat(2 * 1024 * 1024));
    assert_eq!(target(&policy, &huge), Err(TOO_LONG));
    for url in [
        String::new(),
        "about".into(),
        "not a url".into(),
        "1http://x/".into(),
        ":x".into(),
        "https://e.org/a\tb".into(),
        "https://e.org/\u{85}".into(),
        format!("{main}a\u{1}b.html"),
    ] {
        assert_eq!(target(&policy, &url), Err(NO_ADDRESS), "{url:?}");
    }
}

#[test]
fn hover_reports_only_changes_and_an_empty_target_when_the_pointer_leaves() {
    let policy = policy();
    let main = policy.main_url();
    let mut links = Links::default();
    let now = Instant::now();
    assert!(links.hover(&policy, "", now).is_empty(), "no link at first");
    let link = format!("{main}sub/a.html#x");
    assert_eq!(links.hover(&policy, &link, now), [hovered("sub/a.html#x")]);
    assert!(links.hover(&policy, &link, now).is_empty());
    assert_eq!(
        links.hover(&policy, "https://example.org/", now),
        [hovered("https://example.org/")]
    );
    assert_eq!(links.hover(&policy, "", now), [hovered("")]);
    assert!(links.hover(&policy, "", now).is_empty());
    let up = resolved(&main, "../up.html");
    assert_eq!(links.hover(&policy, &up, now), [hovered("../up.html")]);
}

#[test]
fn hovering_a_link_the_viewer_cannot_describe_clears_the_target_with_a_notice() {
    let policy = policy();
    let main = policy.main_url();
    let mut links = Links::default();
    let start = Instant::now();
    assert_eq!(
        links.hover(&policy, &format!("{main}a.html"), start),
        [hovered("a.html")]
    );
    let above = resolved(&main, &format!("{}x.html", "../".repeat(17)));
    assert_eq!(
        links.hover(&policy, &above, start),
        [hovered(""), denied(ABOVE)]
    );
    assert!(
        links.hover(&policy, &above, start).is_empty(),
        "the same link"
    );
    assert!(
        links.hover(&policy, "", start).is_empty(),
        "the host shows no target already"
    );
    let too_long = format!("https://e.org/{}", "x".repeat(api::MAX_LINK_TARGET_BYTES));
    let soon = start + Duration::from_millis(10);
    assert!(
        links.hover(&policy, &too_long, soon).is_empty(),
        "one notice per interval"
    );
    assert!(links.hover(&policy, "", soon).is_empty());
    let later = start + NOTICE_INTERVAL;
    assert_eq!(links.hover(&policy, &too_long, later), [denied(TOO_LONG)]);
}

#[test]
fn a_followed_link_reaches_the_host_once_per_interval() {
    let policy = policy();
    let main = policy.main_url();
    let mut links = Links::default();
    let start = Instant::now();
    assert_eq!(
        links.activate(&policy, &format!("{main}sub/b.html"), start),
        Some(activated("sub/b.html"))
    );
    // The second click of a double click, or a script navigating again on one gesture.
    let soon = start + ACTIVATION_INTERVAL - Duration::from_millis(1);
    assert_eq!(
        links.activate(&policy, &format!("{main}c.html"), soon),
        None
    );
    assert_eq!(
        links.activate(&policy, "https://example.org/", start + ACTIVATION_INTERVAL),
        Some(activated("https://example.org/"))
    );
}

#[test]
fn refused_links_and_navigations_get_one_notice_per_interval() {
    let policy = policy();
    let main = policy.main_url();
    let mut links = Links::default();
    let start = Instant::now();
    let above = resolved(&main, &format!("{}x.html", "../".repeat(17)));
    assert_eq!(links.activate(&policy, &above, start), Some(denied(ABOVE)));
    let soon = start + NOTICE_INTERVAL - Duration::from_millis(1);
    assert_eq!(links.refuse(BY_ITSELF, soon), None);
    let later = start + NOTICE_INTERVAL;
    assert_eq!(links.refuse(BY_ITSELF, later), Some(denied(BY_ITSELF)));
    // A refused link does not hold back the next one the user follows.
    assert_eq!(
        links.activate(&policy, &format!("{main}a.html"), later),
        Some(activated("a.html"))
    );
}

#[test]
fn new_windows_and_tabs_open_nothing_and_only_a_users_link_reaches_the_host() {
    let policy = policy();
    let main = policy.main_url();
    let mut links = Links::default();
    let start = Instant::now();
    // target=_blank, window.open, a middle click or a Ctrl/Shift click.
    assert_eq!(
        links.open_elsewhere(&policy, &format!("{main}new.html"), true, start),
        Some(activated("new.html"))
    );
    let next = start + ACTIVATION_INTERVAL;
    assert_eq!(
        links.open_elsewhere(&policy, "https://example.org/ad", false, next),
        Some(denied(BY_ITSELF))
    );
    let later = next + NOTICE_INTERVAL;
    assert_eq!(
        links.open_elsewhere(&policy, "", true, later),
        Some(denied(NO_ADDRESS))
    );
}

#[test]
fn notices_name_the_limits_they_enforce_and_fit_the_hosts_status_line() {
    assert!(ABOVE.contains(&SYNTHETIC_LEVELS.to_string()));
    assert!(TOO_LONG.contains(&api::MAX_LINK_TARGET_BYTES.to_string()));
    for message in [BY_ITSELF, FORM, ABOVE, TOO_LONG, NO_ADDRESS] {
        // The viewer engine shows Denied notices of at most 512 bytes verbatim.
        assert!(!message.is_empty() && message.len() <= 512, "{message}");
        assert!(!message.chars().any(char::is_control), "{message}");
    }
    let distinct: std::collections::HashSet<_> = [BY_ITSELF, FORM, ABOVE, TOO_LONG, NO_ADDRESS]
        .into_iter()
        .collect();
    assert_eq!(distinct.len(), 5);
}
