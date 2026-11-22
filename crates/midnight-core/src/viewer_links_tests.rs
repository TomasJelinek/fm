//! The link policy table (`viewer_links.rs`): what each kind of target becomes, before and
//! after the caller looked at the file system.
use super::*;

fn relative(up: usize, names: &[&str], fragment: Option<&str>) -> LinkTarget {
    LinkTarget::Local(LocalLink {
        path: LocalPath::Relative {
            up,
            names: names.iter().map(|name| (*name).to_owned()).collect(),
        },
        fragment: fragment.map(str::to_owned),
    })
}

fn rooted(drive: Option<char>, names: &[&str]) -> LinkTarget {
    LinkTarget::Local(LocalLink {
        path: LocalPath::Rooted {
            drive,
            names: names.iter().map(|name| (*name).to_owned()).collect(),
        },
        fragment: None,
    })
}

fn external(text: &str) -> LinkTarget {
    LinkTarget::External(ExternalUrl::parse(text).unwrap())
}

#[test]
fn fragments_stay_in_the_document_percent_decoded() {
    for (target, fragment) in [
        ("#installation", "installation"),
        ("#", ""),
        ("#caf%C3%A9", "café"),
        ("#a?b", "a?b"),
        ("#100%", "100%"),
    ] {
        assert_eq!(
            classify_link(target),
            LinkTarget::Fragment(fragment.into()),
            "{target}"
        );
    }
}

#[test]
fn relative_paths_resolve_dot_segments_and_keep_the_fragment_for_the_target() {
    for (target, expected) in [
        ("other.md", relative(0, &["other.md"], None)),
        ("./docs/guide.md", relative(0, &["docs", "guide.md"], None)),
        ("docs/../README.md", relative(0, &["README.md"], None)),
        ("../README.md", relative(1, &["README.md"], None)),
        ("../../a/./b/../c.md", relative(2, &["a", "c.md"], None)),
        ("docs/", relative(0, &["docs"], None)),
        (".", relative(0, &[], None)),
        ("..", relative(1, &[], None)),
        (
            "guide.md#install",
            relative(0, &["guide.md"], Some("install")),
        ),
        ("guide.md?plain=1#x", relative(0, &["guide.md"], Some("x"))),
        ("my%20notes.md", relative(0, &["my notes.md"], None)),
        (
            "%C5%BElu%C5%A5ou%C4%8Dk%C3%BD.md",
            relative(0, &["žluťoučký.md"], None),
        ),
        ("100%.md", relative(0, &["100%.md"], None)),
        ("caf%c3%a9/x.md", relative(0, &["café", "x.md"], None)),
    ] {
        assert_eq!(classify_link(target), expected, "{target}");
    }
}

#[test]
fn encoded_separators_nul_and_broken_utf8_never_reach_a_path() {
    for target in ["a%2Fb.md", "a%00b.md", "a%FFb.md", "%2E%2E%2Fsecret"] {
        assert_eq!(
            classify_link(target),
            LinkTarget::Refused(LinkRefusal::Unreadable),
            "{target}"
        );
    }
}

#[test]
fn relative_links_resolve_against_the_documents_folder() {
    let folder = Path::new(if cfg!(windows) {
        r"C:\docs\guide"
    } else {
        "/docs/guide"
    });
    for (target, expected) in [
        ("x.md", ["docs", "guide", "x.md"].as_slice()),
        ("../x.md", ["docs", "x.md"].as_slice()),
        ("../../../../x.md", ["x.md"].as_slice()),
        ("sub/", ["docs", "guide", "sub"].as_slice()),
    ] {
        let LinkTarget::Local(link) = classify_link(target) else {
            panic!("{target} is local");
        };
        let mut path = PathBuf::from(if cfg!(windows) { r"C:\" } else { "/" });
        path.extend(expected);
        assert_eq!(link.resolve(folder), path, "{target}");
    }
}

#[test]
fn file_urls_without_a_host_are_local_and_with_one_are_refused() {
    if cfg!(windows) {
        assert_eq!(
            classify_link("file:///C:/Users/me/notes.md"),
            rooted(Some('C'), &["Users", "me", "notes.md"])
        );
        assert_eq!(
            classify_link("FILE:///d:/a%20b/c.md"),
            rooted(Some('D'), &["a b", "c.md"])
        );
    } else {
        assert_eq!(
            classify_link("file:///home/me/notes.md"),
            rooted(None, &["home", "me", "notes.md"])
        );
    }
    assert_eq!(classify_link("file:/x/y.md"), rooted(None, &["x", "y.md"]));
    for target in [
        "file://server/share/x.md",
        "file://localhost/x.md",
        "file:////server/share/x.md",
        "//server/share/x.md",
    ] {
        assert_eq!(
            classify_link(target),
            LinkTarget::Refused(LinkRefusal::OtherComputer),
            "{target}"
        );
    }
    assert_eq!(
        classify_link("file:x.md"),
        LinkTarget::Refused(LinkRefusal::Unreadable)
    );
}

#[test]
fn web_and_mail_addresses_go_to_the_system_handler() {
    for target in [
        "https://example.com/a?b=c#d",
        "http://example.com",
        "HTTPS://Example.com/",
        "mailto:someone@example.com",
        "mailto:a@example.com?subject=Hello%20there",
    ] {
        assert!(
            matches!(classify_link(target), LinkTarget::External(_)),
            "{target}"
        );
    }
    assert_eq!(
        classify_link("HTTPS://Example.com/"),
        external("https://Example.com/")
    );
}

#[test]
fn every_other_scheme_is_refused_by_name() {
    for (target, scheme) in [
        ("javascript:alert(1)", "javascript"),
        ("JavaScript:alert(1)", "javascript"),
        ("data:text/html,<b>x</b>", "data"),
        ("vbscript:msgbox", "vbscript"),
        ("ms-settings:privacy", "ms-settings"),
        ("ftp://example.com/x", "ftp"),
        ("tel:+420123456789", "tel"),
        ("notes.md:hidden", "notes.md"),
    ] {
        assert_eq!(
            classify_link(target),
            LinkTarget::Refused(LinkRefusal::Scheme(scheme.into())),
            "{target}"
        );
    }
    assert_eq!(
        LinkRefusal::Scheme("javascript".into()).message(),
        "Links to javascript: addresses are not followed."
    );
}

#[test]
fn empty_oversized_and_control_character_targets_are_refused() {
    assert_eq!(classify_link(""), LinkTarget::Refused(LinkRefusal::Empty));
    let long = format!("{}.md", "a".repeat(MAX_LINK_TARGET_BYTES - 3));
    assert_eq!(long.len(), MAX_LINK_TARGET_BYTES);
    assert!(matches!(classify_link(&long), LinkTarget::Local(_)));
    assert_eq!(
        classify_link(&format!("a{long}")),
        LinkTarget::Refused(LinkRefusal::TooLong)
    );
    for target in ["a\nb.md", "x\u{7f}.md", "https://example.com/\u{85}"] {
        assert_eq!(
            classify_link(target),
            LinkTarget::Refused(LinkRefusal::ControlCharacter),
            "{target:?}"
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_devices_streams_shares_and_drive_forms_are_refused_or_rooted() {
    for target in [
        "CON",
        "nul.txt",
        "sub/COM1.md",
        "LPT9",
        "conin$",
        "aux.tar.gz",
        r"\\.\PhysicalDrive0",
        r"\\?\C:\x.md",
        "//./pipe/x",
        "file:///C:/x/con.md",
    ] {
        assert_eq!(
            classify_link(target),
            LinkTarget::Refused(LinkRefusal::Device),
            "{target}"
        );
    }
    assert_eq!(
        classify_link("x%3Ay.md"),
        LinkTarget::Refused(LinkRefusal::Device),
        "a decoded colon names a file stream"
    );
    assert_eq!(
        classify_link(r"\\server\share\x.md"),
        LinkTarget::Refused(LinkRefusal::OtherComputer)
    );
    assert_eq!(
        classify_link(r"C:\a\b.md"),
        rooted(Some('C'), &["a", "b.md"])
    );
    assert_eq!(
        classify_link("c:/a/b.md"),
        rooted(Some('C'), &["a", "b.md"])
    );
    assert_eq!(classify_link(r"\a\b.md"), rooted(None, &["a", "b.md"]));
    assert_eq!(
        classify_link(r"docs\guide.md"),
        relative(0, &["docs", "guide.md"], None)
    );
    for target in [
        "a*.md", "a%22b.md", "<x>.md", "a|b.md", "c:x.md", "a:b/c.md", "c:",
    ] {
        assert!(
            matches!(classify_link(target), LinkTarget::Refused(_)),
            "{target}"
        );
    }
    let rooted_link = classify_link(r"\x\y.md");
    let LinkTarget::Local(link) = rooted_link else {
        panic!("rooted");
    };
    assert_eq!(link.resolve(Path::new(r"D:\docs")), Path::new(r"D:\x\y.md"));
}

#[test]
fn mail_links_never_carry_file_attachments() {
    for (target, expected) in [
        (
            "mailto:a@example.com?attach=/etc/passwd",
            "mailto:a@example.com",
        ),
        (
            "mailto:a@example.com?subject=Hi&Attachment=C:%5Csecret.txt&body=x",
            "mailto:a@example.com?subject=Hi&body=x",
        ),
        (
            "mailto:a@example.com?%61ttach=x&cc=b@example.com",
            "mailto:a@example.com?cc=b@example.com",
        ),
        ("mailto:a@example.com?ATTACHMENTS=x", "mailto:a@example.com"),
    ] {
        let url = ExternalUrl::parse(target).unwrap();
        assert_eq!(url.as_str(), expected, "{target}");
        assert!(url.attachments_removed(), "{target}");
    }
    let plain = ExternalUrl::parse("mailto:a@example.com?subject=attach").unwrap();
    assert_eq!(plain.as_str(), "mailto:a@example.com?subject=attach");
    assert!(!plain.attachments_removed());
}

#[test]
fn external_addresses_are_handed_over_as_plain_ascii_that_cannot_break_quoting() {
    for (target, expected) in [
        (
            "https://example.com/a b\"c<d>",
            "https://example.com/a%20b%22c%3Cd%3E",
        ),
        (
            "https://cs.wikipedia.org/wiki/Žluťoučký",
            "https://cs.wikipedia.org/wiki/%C5%BDlu%C5%A5ou%C4%8Dk%C3%BD",
        ),
        ("https://example.com/100%", "https://example.com/100%25"),
        ("https://example.com/%41", "https://example.com/%41"),
        (
            "mailto:Jan Novák <jan@example.com>",
            "mailto:Jan%20Nov%C3%A1k%20%3Cjan@example.com%3E",
        ),
    ] {
        let url = ExternalUrl::parse(target).unwrap();
        assert_eq!(url.as_str(), expected, "{target}");
        assert!(url.as_str().bytes().all(|byte| byte.is_ascii_graphic()));
    }
    for target in [
        "https://",
        "http:///x",
        "https:example.com",
        "http://:80/x",
        "https://user@/x",
    ] {
        assert_eq!(
            ExternalUrl::parse(target),
            Err(LinkRefusal::Unreadable),
            "{target}"
        );
    }
    assert_eq!(
        ExternalUrl::parse(&format!("https://example.com/{}", "é".repeat(2000))),
        Err(LinkRefusal::TooLong),
        "the encoded form stays within the bound"
    );
}

#[test]
fn the_viewers_private_address_is_not_sent_to_the_browser() {
    assert_eq!(
        classify_link("https://mewv-3.invalid/mewv-document/x.html"),
        LinkTarget::Refused(LinkRefusal::PrivateAddress)
    );
    assert_eq!(
        classify_link("https://example.invalid/"),
        LinkTarget::Refused(LinkRefusal::PrivateAddress)
    );
}

#[test]
fn a_local_target_opens_shows_or_reveals_but_never_runs() {
    let path = PathBuf::from(if cfg!(windows) {
        r"C:\d\tool.exe"
    } else {
        "/d/tool"
    });
    assert_eq!(
        local_link_action(
            path.clone(),
            None,
            LocalTargetKind::File { viewable: false }
        ),
        LinkAction::Reveal(path.clone()),
        "a program is revealed in its folder, never started"
    );
    assert_eq!(
        local_link_action(
            path.clone(),
            Some("x".into()),
            LocalTargetKind::File { viewable: true }
        ),
        LinkAction::OpenInViewer {
            path: path.clone(),
            fragment: Some("x".into())
        }
    );
    assert_eq!(
        local_link_action(path.clone(), None, LocalTargetKind::Folder),
        LinkAction::OpenFolder(path.clone())
    );
    let LinkAction::Refuse(missing) =
        local_link_action(path.clone(), None, LocalTargetKind::Missing)
    else {
        panic!("a missing target is refused");
    };
    assert!(missing.ends_with("does not exist."), "{missing}");
    let LinkAction::Refuse(device) = local_link_action(path, None, LocalTargetKind::Other) else {
        panic!("a device or pipe is refused");
    };
    assert!(device.contains("not a file or folder"), "{device}");
}

#[test]
fn the_status_line_names_the_real_target() {
    let folder = Path::new(if cfg!(windows) { r"C:\docs" } else { "/docs" });
    let other = if cfg!(windows) {
        r"C:\other.md"
    } else {
        "/other.md"
    };
    assert_eq!(
        describe_link("../other.md", Some(folder)),
        format!("Link to {other}")
    );
    assert_eq!(
        describe_link("#install", Some(folder)),
        "Link to #install in this document"
    );
    assert_eq!(
        describe_link("https://example.com/a b", Some(folder)),
        "Link to https://example.com/a%20b"
    );
    assert_eq!(
        describe_link("javascript:alert(1)", Some(folder)),
        "Link to javascript:alert(1) · Links to javascript: addresses are not followed."
    );
    assert_eq!(describe_link("x.md", None), "Link to x.md");
    let long = describe_link(&format!("https://example.com/{}", "a".repeat(2000)), None);
    assert!(long.chars().count() <= 300 && long.ends_with('…'), "{long}");
    assert_eq!(
        describe_link("evil\u{202e}gnp.md", Some(folder)),
        format!(
            "Link to {}",
            if cfg!(windows) {
                r"C:\docs\evil\u202Egnp.md"
            } else {
                r"/docs/evil\u202Egnp.md"
            }
        )
    );
}
