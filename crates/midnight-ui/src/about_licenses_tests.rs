use super::*;
use std::collections::HashSet;

/// Each shipped scope's own lockfile, as `docs/THIRD_PARTY.md` lists the manifests.
const LOCKFILES: [(&str, &str); 14] = [
    ("app", include_str!("../../../Cargo.lock")),
    (
        "native-linux",
        include_str!("../../../plugins/midnight-filesystem-native-linux/Cargo.lock"),
    ),
    (
        "native-windows",
        include_str!("../../../plugins/midnight-filesystem-native-windows/Cargo.lock"),
    ),
    (
        "network",
        include_str!("../../../plugins/midnight-filesystem-network/Cargo.lock"),
    ),
    (
        "archive",
        include_str!("../../../plugins/midnight-fs-archive/Cargo.lock"),
    ),
    (
        "basic",
        include_str!("../../../plugins/midnight-viewer-basic/Cargo.lock"),
    ),
    (
        "source",
        include_str!("../../../plugins/midnight-viewer-source/Cargo.lock"),
    ),
    (
        "markdown",
        include_str!("../../../plugins/midnight-viewer-markdown/Cargo.lock"),
    ),
    (
        "html-viewer",
        include_str!("../../../plugins/midnight-viewer-html/Cargo.lock"),
    ),
    (
        "image-viewer",
        include_str!("../../../plugins/midnight-viewer-image/Cargo.lock"),
    ),
    (
        "image-decoder",
        include_str!("../../../plugins/image-decoder/Cargo.lock"),
    ),
    (
        "pdf-viewer",
        include_str!("../../../plugins/midnight-viewer-pdf/Cargo.lock"),
    ),
    (
        "thumbnail-image",
        include_str!("../../../plugins/midnight-thumbnail-image/Cargo.lock"),
    ),
    (
        "thumbnail-video",
        include_str!("../../../plugins/midnight-thumbnail-video/Cargo.lock"),
    ),
];

fn locked(lock: &str) -> HashSet<(String, String)> {
    let mut packages = HashSet::new();
    let mut name = None;
    for line in lock.lines() {
        if let Some(value) = line.strip_prefix("name = ") {
            name = Some(value.trim_matches('"').to_owned());
        } else if let Some(value) = line.strip_prefix("version = ")
            && let Some(name) = name.take()
        {
            packages.insert((name, value.trim_matches('"').to_owned()));
        }
    }
    packages
}

fn find<'a>(libraries: &'a [Library], name: &str, version: &str) -> Option<&'a Library> {
    libraries
        .iter()
        .find(|library| library.name == name && library.version == version)
}

#[test]
fn the_list_is_the_current_inventory_restricted_to_what_ships() {
    assert_eq!(
        fields(INVENTORY.lines().next().unwrap()),
        COLUMNS,
        "the About list reads these inventory columns"
    );
    for target in ["Linux", "Windows"] {
        let libraries = libraries_for(INVENTORY, target);
        assert!(libraries.len() > 400, "{target}: {} rows", libraries.len());
        let mut seen = HashSet::new();
        for library in &libraries {
            assert!(
                !library.name.is_empty()
                    && !library.license.is_empty()
                    && !library.used_by.is_empty(),
                "{library:?}"
            );
            assert!(
                seen.insert((library.name.clone(), library.version.clone())),
                "repeated {library:?}"
            );
        }
        // Libraries the application uses now, which the old audit snapshot lacked.
        for (name, version) in [
            ("accesskit", "0.24.1"),
            ("accesskit_consumer", "0.38.0"),
            ("sha2", "0.10.9"),
            ("md-5", "0.10.6"),
            ("blake3", "1.8.7"),
            ("zip", "8.6.0"),
            ("toml", "0.9.12+spec-1.1.0"),
            ("slint", "1.18.1"),
        ] {
            let library = find(&libraries, name, version)
                .unwrap_or_else(|| panic!("{target}: {name} {version} missing"));
            assert!(library.used_by.contains("Midnight Explorer"), "{library:?}");
        }
        // Plugin libraries name the plugin that ships them.
        let hayro = find(&libraries, "hayro", "0.9.0").unwrap();
        assert_eq!(hayro.used_by, "PDF viewer, image thumbnails");
        let image = find(&libraries, "image", "0.25.10").unwrap();
        assert!(
            image.used_by.contains("Midnight Explorer") && image.used_by.contains("image viewer")
        );
        // Test-only and SDK-only libraries, and libraries no longer used, are left out.
        for (name, version) in [
            ("cbindgen", "0.29.4"),
            ("cosmic-text", "0.19.0"),
            ("fs2", "0.4.3"),
            ("winapi", "0.3.9"),
        ] {
            assert!(
                find(&libraries, name, version).is_none(),
                "{target}: {name}"
            );
        }
        // Components outside Cargo.
        for (name, version) in [
            ("FFmpeg", "9.0.2"),
            ("dav1d", "1.5.4"),
            ("Chromium Embedded Framework", "156.0.26"),
            ("Noto Sans fonts", ""),
            ("Rust standard library", "1.99.1"),
        ] {
            assert!(
                find(&libraries, name, version).is_some(),
                "{target}: {name}"
            );
        }
    }
    let linux = libraries_for(INVENTORY, "Linux");
    let windows = libraries_for(INVENTORY, "Windows");
    assert!(find(&linux, "atspi", "0.29.0").is_some());
    assert!(find(&windows, "atspi", "0.29.0").is_none());
    assert!(find(&windows, "accesskit_windows", "0.34.0").is_some());
    assert!(find(&linux, "accesskit_windows", "0.34.0").is_none());
    assert_eq!(libraries(), libraries_for(INVENTORY, TARGET));
    // Another column layout lists no Cargo library rather than misreading one.
    let reordered = INVENTORY.replacen("name,version,license", "version,name,license", 1);
    assert_eq!(libraries_for(&reordered, "Linux").len(), BUNDLED.len());
}

#[test]
fn every_listed_library_is_pinned_by_the_lockfile_of_each_component_that_ships_it() {
    let locks: HashMap<&str, HashSet<(String, String)>> = LOCKFILES
        .iter()
        .map(|(scope, lock)| (*scope, locked(lock)))
        .collect();
    assert_eq!(locks.len(), SHIPPED.len());
    let mut checked = 0;
    for line in INVENTORY.lines().skip(1) {
        let columns = fields(line);
        let [name, version, _license, _source, scopes, _targets, _direct] = columns.as_slice()
        else {
            panic!("malformed inventory row {line}")
        };
        for scope in scopes.split("; ") {
            let Some(lock) = locks.get(scope) else {
                continue;
            };
            checked += 1;
            assert!(
                lock.contains(&(name.clone(), version.clone())),
                "the inventory is out of date: {scope} no longer pins {name} {version}; \
                 regenerate docs/dependencies (docs/THIRD_PARTY.md)"
            );
        }
    }
    assert!(checked > 500, "{checked}");
}

#[test]
fn bundled_components_match_their_records() {
    let third_party = include_str!("../../../docs/THIRD_PARTY.md");
    let themes = include_str!("../../../docs/THEME_PROVENANCE.md");
    let records = [
        (
            "Rust standard library",
            include_str!("../../../rust-toolchain.toml"),
            "channel = \"1.99.1\"",
        ),
        ("FFmpeg", third_party, "FFmpeg 9.0.2 shared libraries"),
        ("dav1d", third_party, "native dav1d 1.5.4"),
        (
            "Chromium Embedded Framework",
            third_party,
            "`156.0.26+g8ea0c73",
        ),
        (
            "Noto Sans fonts",
            third_party,
            "Noto Sans Regular/Arabic/Hebrew font bytes are embedded under OFL-1.1",
        ),
        (
            "Unicode case-folding data",
            include_str!("../../../plugins/midnight-viewer-basic/THIRD-PARTY.md"),
            "Unicode 17.0.0 data",
        ),
        ("Foxit standard PDF fonts", INVENTORY, "\nhayro-font,0.5.0,"),
        ("Adobe CMaps", INVENTORY, "\nhayro-cmap,0.1.1,"),
        (
            "Tokyo Night palette",
            themes,
            "commit cdc07ac](https://github.com/folke/",
        ),
        (
            "One Dark palette",
            themes,
            "commit 47bec7a](https://github.com/joshdick/",
        ),
        (
            "Rosé Pine palette",
            themes,
            "commit ff48305](https://github.com/rose-pine/",
        ),
        (
            "Catppuccin palette",
            themes,
            "commit d09787d](https://github.com/catppuccin/",
        ),
    ];
    assert_eq!(records.len(), BUNDLED.len());
    for ((name, record, needle), bundled) in records.iter().zip(&BUNDLED) {
        assert_eq!(*name, bundled.name);
        assert!(
            record.contains(needle),
            "{name}: the record no longer says {needle:?}"
        );
        if let Some(folder) = bundled.folder
            && let Some(author) = folder.strip_prefix("themes/")
        {
            // The release collector copies exactly these palette licenses.
            assert!(
                include_str!("../../../scripts/package-release.py")
                    .contains(&format!("\"{author}\"")),
                "{name}: package-release.py no longer ships licenses/{folder}"
            );
        }
    }
}

#[test]
fn quoted_fields_keep_their_commas_and_filters_match_name_license_or_component() {
    assert_eq!(fields(r#"a,"b,c","d""e""#), ["a", "b,c", r#"d"e"#]);
    let library = |name: &str, license: &str, used_by: &str| Library {
        name: name.into(),
        version: "1".into(),
        license: license.into(),
        used_by: used_by.into(),
        bundled: None,
    };
    let rows = [
        library("Serde", "MIT OR Apache-2.0", "Midnight Explorer"),
        library("zlib", "Zlib", "PDF viewer"),
    ];
    assert_eq!(matching(&rows, " ser "), [0]);
    assert_eq!(matching(&rows, "SERDE"), [0]);
    assert_eq!(matching(&rows, "zlib"), [1]);
    assert_eq!(matching(&rows, "mit"), [0]);
    assert_eq!(matching(&rows, "pdf"), [1]);
    assert_eq!(matching(&rows, ""), [0, 1]);
}

#[test]
fn only_the_awaited_answer_is_kept() {
    let mut state = NoticeState::default();
    let first = state.begin(3);
    let second = state.begin(4);
    assert_ne!(first, second);
    state.receive(Box::new(LicenseNotice {
        id: first,
        text: "old".into(),
        ..LicenseNotice::default()
    }));
    assert!(state.answer.is_none());
    state.receive(Box::new(LicenseNotice {
        id: second,
        text: "new".into(),
        ..LicenseNotice::default()
    }));
    assert_eq!(
        state.answer.as_ref().map(|answer| answer.text.as_str()),
        Some("new")
    );
    assert_eq!(state.awaited, Some((second, 4)));
    state.cancel();
    assert!(state.answer.is_none() && state.awaited.is_none());
}

#[test]
fn authors_show_one_line_each_under_a_heading() {
    assert_eq!(authors_block(None), ("", Vec::new()), "loading");
    assert_eq!(
        authors_block(Some(&[])),
        ("Authors: not stated in its license files.", Vec::new())
    );
    let authors = [
        "Marshall Pierce <marshall@mpierce.org>".to_owned(),
        "Copyright (c) 2025 Alice Maz, Marshall Pierce".to_owned(),
    ];
    assert_eq!(
        authors_block(Some(&authors)),
        (
            "Authors",
            vec![
                SharedString::from("Marshall Pierce <marshall@mpierce.org>"),
                SharedString::from("Copyright (c) 2025 Alice Maz, Marshall Pierce"),
            ]
        )
    );
    // A bundled component's authors are separated by semicolons in its record.
    let cef = BUNDLED
        .iter()
        .find(|bundled| bundled.name == "Chromium Embedded Framework")
        .unwrap();
    assert_eq!(
        bundled_authors(cef),
        ["Marshall A. Greenblatt", "portions Google Inc."]
    );
    let ffmpeg = BUNDLED.iter().find(|bundled| bundled.name == "FFmpeg");
    assert_eq!(
        ffmpeg.map(bundled_authors),
        Some(vec!["The FFmpeg developers".to_owned()])
    );
}

#[test]
fn requests_name_a_package_folder_or_a_bundled_one() {
    let all = libraries_for(INVENTORY, "Linux");
    let zip = find(&all, "zip", "8.6.0").unwrap();
    assert_eq!(
        zip.request(LicenseNoticeId(1)),
        Some(LicenseNoticeRequest {
            id: LicenseNoticeId(1),
            name: "zip".into(),
            version: "8.6.0".into(),
            folder: None,
        })
    );
    let rust = find(&all, "Rust standard library", "1.99.1").unwrap();
    assert_eq!(
        rust.request(LicenseNoticeId(2))
            .and_then(|request| request.folder),
        Some("rust".into())
    );
    // FFmpeg's notices ship with its plugin package: nothing to read beside the program.
    let ffmpeg = find(&all, "FFmpeg", "9.0.2").unwrap();
    assert_eq!(ffmpeg.request(LicenseNoticeId(3)), None);
    assert!(missing_text(ffmpeg).contains("notices/ffmpeg-9.0.2/"));
    assert!(missing_text(zip).contains("The declared license is MIT."));
}

#[test]
fn the_own_license_is_the_complete_gpl_version_3() {
    assert_eq!(OWN_LICENSE, "GPL-3.0-or-later");
    assert_eq!(OWN_COPYRIGHT, "© 2026 Tomas Jelinek");
    let text = OWN_LICENSE_TEXT;
    assert!(text.trim_start().starts_with("GNU GENERAL PUBLIC LICENSE"));
    assert!(text.contains("Version 3, 29 June 2007"));
    for section in [
        "  0. Definitions.",
        "  15. Disclaimer of Warranty.",
        "  17. Interpretation of Sections 15 and 16.",
    ] {
        assert!(text.contains(section), "{section:?} is missing");
    }
    assert!(text.contains("END OF TERMS AND CONDITIONS"));
    assert!(text.contains("How to Apply These Terms to Your New Programs"));
    assert!(
        text.trim_end()
            .ends_with("<https://www.gnu.org/licenses/why-not-lgpl.html>.")
    );
    // The unmodified text: LF line endings, no tabs, nothing appended.
    assert!(!text.contains('\r') && !text.contains('\t'));
    assert_eq!(text.lines().count(), 674);
}
