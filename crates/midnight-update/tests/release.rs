//! Choosing the release to offer: semver order, previews, drafts, skipped versions.
use midnight_update::{
    notes,
    release::{Finding, newest, parse_releases, parse_tag},
};
use semver::Version;

fn listing() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!([
        {"tag_name": "v0.10.0-beta.1", "draft": false, "prerelease": true, "html_url": "p", "assets": []},
        {"tag_name": "v0.9.0", "draft": false, "prerelease": false, "html_url": "p", "assets": []},
        {"tag_name": "v0.9.1", "draft": false, "prerelease": true, "html_url": "p", "assets": []},
        {"tag_name": "v1.0.0", "draft": true, "prerelease": false, "html_url": "p", "assets": []},
        {"tag_name": "nightly", "draft": false, "prerelease": true, "html_url": "p", "assets": []},
        {"tag_name": "v0.5.0", "draft": false, "prerelease": true, "html_url": "p", "assets": []},
    ]))
    .unwrap()
}

fn version(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn offered(finding: &Finding) -> Option<String> {
    match finding {
        Finding::UpToDate => None,
        Finding::Newer { release, .. } => Some(release.version.to_string()),
    }
}

#[test]
fn drafts_and_tags_that_are_not_versions_are_left_out() {
    let releases = parse_releases(&listing()).unwrap();
    let tags: Vec<_> = releases
        .iter()
        .map(|release| release.tag.as_str())
        .collect();
    assert_eq!(tags, ["v0.10.0-beta.1", "v0.9.0", "v0.9.1", "v0.5.0"]);
}

#[test]
fn the_newest_by_semver_is_offered_with_previews() {
    let releases = parse_releases(&listing()).unwrap();
    // 0.10.0-beta.1 is newer than 0.9.1 by semver, although "0.9" sorts after "0.1" as text.
    let finding = newest(&releases, &version("0.5.0"), true, None);
    assert_eq!(offered(&finding).as_deref(), Some("0.10.0-beta.1"));
}

#[test]
fn without_previews_only_stable_releases_are_offered() {
    let releases = parse_releases(&listing()).unwrap();
    let finding = newest(&releases, &version("0.5.0"), false, None);
    // 0.9.1 is marked as a pre-release on GitHub, 0.10.0-beta.1 has a semver pre-release.
    assert_eq!(offered(&finding).as_deref(), Some("0.9.0"));
}

#[test]
fn nothing_newer_is_up_to_date() {
    let releases = parse_releases(&listing()).unwrap();
    assert_eq!(
        newest(&releases, &version("0.10.0"), true, None),
        Finding::UpToDate
    );
    // The running pre-release itself is not newer than itself.
    assert_eq!(
        newest(&releases, &version("0.10.0-beta.1"), true, None),
        Finding::UpToDate
    );
    assert_eq!(
        newest(&[], &version("0.1.0"), true, None),
        Finding::UpToDate
    );
}

#[test]
fn a_skipped_version_is_marked_but_a_newer_one_is_not() {
    let releases = parse_releases(&listing()).unwrap();
    let skipped = version("0.10.0-beta.1");
    match newest(&releases, &version("0.5.0"), true, Some(&skipped)) {
        Finding::Newer { skipped, release } => {
            assert!(skipped);
            assert_eq!(release.version, version("0.10.0-beta.1"));
        }
        Finding::UpToDate => panic!("expected an offer"),
    }
    match newest(&releases, &version("0.5.0"), true, Some(&version("0.9.1"))) {
        Finding::Newer { skipped, .. } => assert!(!skipped),
        Finding::UpToDate => panic!("expected an offer"),
    }
}

#[test]
fn tags_parse_with_or_without_v() {
    assert_eq!(parse_tag("v0.5.1"), Some(version("0.5.1")));
    assert_eq!(parse_tag("0.5.1"), Some(version("0.5.1")));
    assert_eq!(parse_tag("v0.6.0-rc.1"), Some(version("0.6.0-rc.1")));
    assert_eq!(parse_tag("release-1"), None);
}

#[test]
fn a_listing_that_is_not_an_array_is_refused() {
    assert!(parse_releases(br#"{"message":"Not Found"}"#).is_err());
}

#[test]
fn missing_files_name_the_reason() {
    let platform = crate::support::platform();
    let releases = parse_releases(&listing()).unwrap();
    let reason = releases[0].missing_files(&platform).unwrap();
    assert!(reason.contains("SHA256SUMS"), "{reason}");
}

#[test]
fn release_notes_become_plain_text() {
    let (text, cut) = notes::excerpt(
        "## What's new\n\n- **Faster** `scrolling`\n- See [the guide](https://x.invalid/g)\n\n---\n\nThanks",
    );
    assert_eq!(
        text,
        "What's new\n\n• Faster scrolling\n• See the guide\n\nThanks"
    );
    assert!(!cut);
    let long = "line of release notes\n".repeat(400);
    let (text, cut) = notes::excerpt(&long);
    assert!(cut);
    assert!(text.chars().count() <= notes::EXCERPT_CHARACTERS);
    assert!(text.ends_with("notes"));
}
