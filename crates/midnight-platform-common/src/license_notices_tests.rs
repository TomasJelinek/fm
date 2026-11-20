use super::*;
use midnight_core::LicenseNoticeId;

#[cfg(unix)]
#[path = "license_notices_unix_tests.rs"]
mod unix;

fn request(name: &str, version: &str, folder: Option<&str>) -> LicenseNoticeRequest {
    LicenseNoticeRequest {
        id: LicenseNoticeId(7),
        name: name.into(),
        version: version.into(),
        folder: folder.map(str::to_owned),
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn package(root: &Path) -> NoticeSources {
    NoticeSources {
        installed: Some(root.join("licenses")),
        inventory: Some(root.join("THIRD-PARTY.json")),
        registries: Vec::new(),
    }
}

#[test]
fn a_release_folder_yields_every_notice_and_the_recorded_authors() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("licenses/demo-1.2.3+build");
    write(
        &folder.join("LICENSE-MIT"),
        "MIT License\n\nCopyright (c) 2020 Ada Lovelace\n",
    );
    write(
        &folder.join("bundled-notices/api/gl.xml.notice"),
        "Copyright 2013-2020 The Khronos Group Inc.\n",
    );
    write(
        &root.path().join("THIRD-PARTY.json"),
        r#"[{"name": "demo", "version": "1.2.3+build", "license": "MIT",
             "authors": ["Ada <ada@example.org>", "Grace", "Ada <ada@example.org>"]},
            {"name": "other", "version": "1.0.0", "authors": null}]"#,
    );
    let sources = package(root.path());
    let index = AuthorIndex::load(sources.inventory.as_deref());
    let notice = read_notice(&sources, &index, &request("demo", "1.2.3+build", None));
    assert_eq!(notice.id, LicenseNoticeId(7));
    let mit = notice.text.find("──── LICENSE-MIT ────").unwrap();
    let nested = notice
        .text
        .find(&format!(
            "──── {} ────",
            Path::new("bundled-notices")
                .join("api")
                .join("gl.xml.notice")
                .display()
        ))
        .unwrap();
    assert!(mit < nested, "notices are in path order: {}", notice.text);
    assert!(notice.text.contains("The Khronos Group") && !notice.truncated);
    // Manifest authors win over the notices' copyright lines.
    assert_eq!(notice.authors, ["Ada <ada@example.org>", "Grace"]);
}

#[test]
fn without_manifest_authors_only_concrete_copyright_statements_are_authors() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("licenses/demo-0.1.0");
    write(
        &folder.join("LICENSE-APACHE"),
        "      2. Grant of Copyright License. Subject to the terms ...\n\
         copyright license to reproduce, prepare Derivative Works of,\n\
         Copyright [yyyy] [name of copyright owner]\n\
         COPYRIGHT AND PERMISSION NOTICE\n\
         Copyright 2019 The Demo Authors\n",
    );
    write(
        &folder.join("LICENSE-MIT"),
        "Copyright (c) <year> <copyright holders>\n\
         // Copyright (c) 2008-2020 Marshall A. Greenblatt. Portions Copyright (c)\n\
         Copyright 2019 The Demo Authors\n\
         © 2024 Example Ltd.\n",
    );
    write(
        &folder.join("COPYING.LGPLv2.1"),
        " Copyright (C) 1991, 1999 Free Software Foundation, Inc.\n\
         Copyright (C) <year>  <name of author>\n",
    );
    let sources = package(root.path());
    let notice = read_notice(
        &sources,
        &AuthorIndex::default(),
        &request("demo", "0.1.0", None),
    );
    assert_eq!(
        notice.authors,
        [
            "Copyright 2019 The Demo Authors",
            "Copyright (c) 2008-2020 Marshall A. Greenblatt. Portions Copyright (c)",
            "© 2024 Example Ltd.",
        ]
    );
}

/// The appendix of the Apache License 2.0 text, which the vendored image
/// crate's copy leaves out: its template is the line that most often posed as
/// an author.
const APACHE_APPENDIX: &str = r#"
APPENDIX: How to apply the Apache License to your work.

   To apply the Apache License to your work, attach the following
   boilerplate notice, with the fields enclosed by brackets "[]"
   replaced with your own identifying information. (Don't include
   the brackets!)  The text should be enclosed in the appropriate
   comment syntax for the file format. We also recommend that a
   file or class name and description of purpose be included on the
   same "printed page" as the copyright notice for easier
   identification within third-party archives.

Copyright [yyyy] [name of copyright owner]

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
"#;

fn apache_text() -> String {
    format!(
        "{}{APACHE_APPENDIX}",
        include_str!("../../../vendor/image/LICENSE-APACHE")
    )
}

#[test]
fn license_texts_and_their_templates_name_no_author() {
    let apache = apache_text();
    for prose in [
        "copyright notice that is included in or attached to the work",
        "copyright license to reproduce, prepare Derivative Works of,",
        "(c) You must retain, in the Source form of any Derivative Works",
        "Copyright [yyyy] [name of copyright owner]",
    ] {
        assert!(apache.contains(prose), "the Apache text has {prose:?}");
    }
    assert_eq!(copyright_lines(&apache), Vec::<String>::new());
    for template in [
        "Copyright (c) <year> <copyright holders>",
        "Copyright (c) [year] [fullname]",
        "Copyright <YEAR> <COPYRIGHT HOLDER>",
        "Copyright {yyyy} {name of copyright owner}",
        "Copyright (c) <dates>, <Copyright Holder> (<URL|email>),",
        "Copyright (C) <year>  <name of author>",
        "Copyright (C) 2007 Free Software Foundation, Inc. <https://fsf.org/>",
        "copyright notice and this permission notice appear in all copies.",
        "COPYRIGHT HOLDERS AND CONTRIBUTORS \"AS IS\" AND ANY EXPRESS OR IMPLIED",
    ] {
        assert_eq!(copyright_line(template), None, "{template}");
    }
}

#[test]
fn real_copyright_headers_are_authors_once_each() {
    let headers = "/*\n * Copyright (c) 2014 Alex Crichton\n */\n\
                   Copyright (C) 1995-2017  Jean-loup Gailly and Mark Adler\n\
                   # Copyright © 1991-2024 Unicode, Inc.\n\
                   (c) 2019 Ada Lovelace <ada@example.org>\n\
                   COPYRIGHT (C) 2014 ALEX CRICHTON\n\
                   Copyright (C) 1995-2017 Jean-loup Gailly and Mark Adler\n";
    assert_eq!(
        copyright_lines(headers),
        [
            "Copyright (c) 2014 Alex Crichton",
            "Copyright (C) 1995-2017 Jean-loup Gailly and Mark Adler",
            "Copyright © 1991-2024 Unicode, Inc.",
            "(c) 2019 Ada Lovelace <ada@example.org>",
        ]
    );
    // A dual-licensed crate without manifest authors: only its MIT header.
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("licenses/base64-0.23.1");
    write(&folder.join("LICENSE-APACHE"), &apache_text());
    write(
        &folder.join("LICENSE-MIT"),
        "The MIT License (MIT)\n\nCopyright (c) 2025 Alice Maz, Marshall Pierce\n\n\
         The above copyright notice and this permission notice shall be included in\n\
         all copies or substantial portions of the Software.\n",
    );
    let notice = read_notice(
        &package(root.path()),
        &AuthorIndex::default(),
        &request("base64", "0.23.1", None),
    );
    assert_eq!(
        notice.authors,
        ["Copyright (c) 2025 Alice Maz, Marshall Pierce"]
    );
}

#[test]
fn names_must_be_single_components_and_bundled_folders_stay_in_the_package() {
    let root = tempfile::tempdir().unwrap();
    write(&root.path().join("escape-1.0/LICENSE"), "outside");
    write(&root.path().join("licenses/x/LICENSE"), "nested");
    write(
        &root.path().join("licenses/themes/folke/LICENSE"),
        "Apache License",
    );
    let sources = package(root.path());
    let index = AuthorIndex::default();
    for bad in [
        request("../escape", "1.0", None),
        request("escape", "../1.0", None),
        request("x/..", "1", None),
        request("", "1.0", None),
        request("..", "", None),
        request("demo", "1.0", Some("../escape-1.0")),
        request("demo", "1.0", Some("themes/folke/..")),
        request("demo", "1.0", Some("a/b/c")),
        request("demo", "1.0", Some("/etc")),
        request("demo", "1.0", Some(".")),
    ] {
        let notice = read_notice(&sources, &index, &bad);
        assert!(notice.text.is_empty(), "{bad:?} read {}", notice.text);
    }
    let theme = read_notice(
        &sources,
        &index,
        &request("Tokyo Night", "", Some("themes/folke")),
    );
    assert!(theme.text.contains("Apache License"), "{}", theme.text);
}

#[test]
fn reads_are_capped_before_they_start() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("licenses/huge-1.0.0");
    let line = "Permission is hereby granted, free of charge.\n";
    write(&folder.join("LICENSE"), &line.repeat(64 * 1024));
    for index in 0..80 {
        write(&folder.join(format!("NOTICE-{index:02}")), "notice\n");
    }
    let notice = read_notice(
        &package(root.path()),
        &AuthorIndex::default(),
        &request("huge", "1.0.0", None),
    );
    assert!(notice.truncated);
    assert!(
        notice.text.len() <= MAX_NOTICE_TEXT + 4096,
        "{}",
        notice.text.len()
    );
    assert!(notice.text.matches("────").count() <= 2 * MAX_FILES);
}

#[test]
fn an_unreadable_or_malformed_inventory_records_no_authors() {
    assert!(AuthorIndex::parse(b"not json").0.is_empty());
    assert!(
        AuthorIndex::parse(br#"[{"name": "a", "version": "1", "authors": [1]}]"#)
            .0
            .is_empty()
    );
    assert!(AuthorIndex::load(None).0.is_empty());
    let root = tempfile::tempdir().unwrap();
    assert!(
        AuthorIndex::load(Some(&root.path().join("missing.json")))
            .0
            .is_empty()
    );
    // A folder is not an inventory file.
    assert!(AuthorIndex::load(Some(root.path())).0.is_empty());
}

#[test]
fn release_sources_never_include_a_developer_registry() {
    let root = tempfile::tempdir().unwrap();
    let release = NoticeSources::beside(Some(root.path()), false);
    assert!(release.registries.is_empty());
    assert_eq!(release.installed, Some(root.path().join("licenses")));
    assert_eq!(
        release.inventory,
        Some(root.path().join("THIRD-PARTY.json"))
    );
    let development = NoticeSources::beside(Some(root.path()), true);
    let container = root.path().join(".tools/docker/cargo/registry/src");
    assert!(development.registries.contains(&container));
}

#[test]
fn a_registry_crate_shows_only_its_notices_and_its_manifest_authors() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("registry/src/index.example-1/demo-2.0.0");
    write(&source.join("LICENSE-MIT"), "Copyright (c) 2021 Somebody\n");
    write(&source.join("LICENSES/Apache-2.0.txt"), "Apache text\n");
    write(&source.join("README.md"), "not a notice");
    write(
        &source.join("src/lib.rs"),
        "// Copyright 2021 source file\n",
    );
    write(
        &source.join("Cargo.toml"),
        "[package]\nname = \"demo\"\n# authors = [\"comment\"]\n\
         authors = [\n  \"Ada <ada@example.org>\",\n  \"Grace \\\"G\\\" H\",\n]\n\
         [dependencies]\nauthors = [\"not the package\"]\n",
    );
    let sources = NoticeSources {
        installed: Some(root.path().join("licenses")),
        inventory: None,
        registries: vec![root.path().join("registry/src")],
    };
    let notice = read_notice(
        &sources,
        &AuthorIndex::default(),
        &request("demo", "2.0.0", None),
    );
    assert!(notice.text.contains("Apache text") && notice.text.contains("Somebody"));
    assert!(!notice.text.contains("not a notice") && !notice.text.contains("source file"));
    assert_eq!(notice.authors, ["Ada <ada@example.org>", "Grace \"G\" H"]);
    // A bundled component is never looked up in a registry.
    let bundled = read_notice(
        &sources,
        &AuthorIndex::default(),
        &request("demo", "2.0.0", Some("demo-2.0.0")),
    );
    assert!(bundled.text.is_empty());
}

#[test]
fn html_notices_become_readable_text() {
    let text = html_text(
        "<!DOCTYPE html><html><head><title>Notices</title></head><body>\n\
         <h1>Copyright notices for Demo</h1>\n\n\n\n<p>A &amp; B &lt;c&gt;</p><ul><li>One</li></ul>",
    );
    assert!(!text.contains('<') || text.contains("<c>"), "{text}");
    assert!(text.contains("Copyright notices for Demo") && text.contains("A & B <c>"));
    assert!(!text.contains("\n\n\n"), "{text:?}");
    assert!(text.lines().any(|line| line == "One"), "{text:?}");
}
