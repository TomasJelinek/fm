#[cfg(windows)]
#[path = "filter_expression_names/windows.rs"]
mod native_names;
#[cfg(unix)]
#[path = "filter_expression_names/unix.rs"]
mod native_names;

use midnight_core::{Entry, EntryId, EntryKind};
use midnight_platform_common::filter_expression::{CompiledExpression, DiagnosticKind};
use std::{path::PathBuf, time::UNIX_EPOCH};

fn file() -> Entry {
    let mut entry = Entry::new(EntryId(1), PathBuf::from("sample.RS"), EntryKind::File);
    entry.metadata.size = Some(10 * 1024 * 1024);
    entry.metadata.modified = Some(UNIX_EPOCH);
    entry
}
fn matches(source: &str, entry: &Entry) -> bool {
    CompiledExpression::new(source, false)
        .unwrap()
        .matches(entry)
}

#[test]
fn typed_predicates_compose_with_explicit_precedence_and_name_case_policy() {
    let entry = file();
    assert!(matches(
        "kind = file and size >= 10MiB and name like \"*.rs\"",
        &entry
    ));
    assert!(matches(
        "kind = directory or kind = file and size = 10485760",
        &entry
    ));
    assert!(!matches(
        "(kind = directory or kind = file) and size > 10MiB",
        &entry
    ));
    assert!(matches("name = \"sample.rs\"", &entry));
    assert!(
        !CompiledExpression::new("name = \"sample.rs\"", true)
            .unwrap()
            .matches(&entry)
    );
    assert!(matches("name =~ \"^sample\\\\.RS$\"", &entry));
    assert!(matches("size > 10MB and size < 11MiB", &entry));
}

#[test]
fn unknown_metadata_has_three_valued_logic_including_negation() {
    let entry = file();
    assert!(!matches("attributes has hidden", &entry));
    assert!(!matches("not attributes has hidden", &entry));
    assert!(matches(
        "missing(attributes) or not attributes has hidden",
        &entry
    ));
    assert!(!matches("present(attributes) and kind = file", &entry));
    assert!(matches("kind = file or attributes has hidden", &entry));
    assert!(!matches(
        "not (kind = directory or attributes has hidden)",
        &entry
    ));
    assert!(!matches(
        "kind = directory and attributes has hidden",
        &entry
    ));
    assert!(matches("not missing(kind)", &entry));
}

#[test]
fn directory_sizes_and_link_targets_are_never_invented() {
    let mut entry = file();
    for kind in [
        EntryKind::Directory,
        EntryKind::DirectoryLink,
        EntryKind::Symlink,
        EntryKind::Other,
    ] {
        entry.kind = kind;
        assert!(matches("missing(size)", &entry));
        assert!(!matches("size != 0", &entry));
    }
    entry.kind = EntryKind::DirectoryLink;
    assert!(matches(
        "kind = directory-link and not kind = directory",
        &entry
    ));
    entry.metadata.native_attributes = Some(0x2 | 0x1);
    assert!(matches(
        "attributes has hidden and attributes has readonly",
        &entry
    ));
    assert!(!matches("attributes has system", &entry));
}

#[test]
fn opaque_archive_names_never_promote_a_display_caption_into_authoritative_name_text() {
    use midnight_core::{ArchiveNode, VirtualEntryId};
    for raw_name in [
        b"invalid-\xff.txt".as_slice(),
        b"different-name.txt".as_slice(),
    ] {
        let entry = Entry::from_archive(ArchiveNode {
            modified: None,
            id: VirtualEntryId { mount: 9, entry: 2 },
            parent: 1,
            raw_name: raw_name.into(),
            display_name: "caption.txt".into(),
            kind: EntryKind::File,
            size: Some(100),
            readable: true,
        });
        assert!(matches(
            "missing(name) and kind = file and size = 100",
            &entry
        ));
        assert!(!matches("name = \"caption.txt\"", &entry));
        assert!(!matches("name != \"other\"", &entry));
        assert!(!matches("not name like \"*\"", &entry));
        assert!(!matches("present(name)", &entry));
    }
    let entry = Entry::from_archive(ArchiveNode {
        modified: None,
        id: VirtualEntryId { mount: 9, entry: 3 },
        parent: 1,
        raw_name: "文档.rs".as_bytes().into(),
        display_name: "文档.rs".into(),
        kind: EntryKind::File,
        size: Some(42),
        readable: true,
    });
    assert!(matches("present(name) and name like \"*.rs\"", &entry));
}

#[test]
fn timestamp_literals_are_exact_utc_comparisons_with_checked_offsets() {
    let mut entry = file();
    assert!(matches("modified = \"1970-01-01T01:00:00+01:00\"", &entry));
    assert!(matches(
        "modified > \"1969-12-31T23:59:59.999999999Z\"",
        &entry
    ));
    entry.metadata.modified = Some(UNIX_EPOCH + std::time::Duration::from_nanos(100));
    assert!(matches(
        "modified = \"1970-01-01T00:00:00.000000100Z\"",
        &entry
    ));
    assert!(!matches("modified = \"1970-01-01T00:00:00Z\"", &entry));
    assert!(CompiledExpression::new("modified >= \"2000-02-29T00:00:00Z\"", false).is_ok());
    for date in [
        "1900-02-29T00:00:00Z",
        "2026-11-07",
        "2026-11-07T00:00:00",
        "2026-11-07T24:00:00Z",
        "2026-11-07T00:00:60Z",
        "2026-11-07T00:00:00+24:00",
        "2026-11-07T00:00:00.1234567890Z",
    ] {
        assert!(
            CompiledExpression::new(&format!("modified = \"{date}\""), false).is_err(),
            "{date}"
        );
    }
}

#[test]
fn syntax_type_overflow_and_escape_errors_report_valid_utf8_spans() {
    for source in [
        "name = \"é\" trailing",
        "size = \"large\"",
        "kind > file",
        "modified >= 0",
        "attributes = 3",
        "attributes has invented",
        "size > 18446744073709551616",
        "size > 18446744073709551615TiB",
        "size > 1.5MB",
        "name = \"\\q\"",
        "name = \"unfinished",
        "name =~ \"[\"",
        "missing(unknown)",
        "",
        "name = \"x\"\0",
    ] {
        let error = CompiledExpression::new(source, false).err().expect(source);
        assert!(error.span.start <= error.span.end && error.span.end <= source.len());
        assert!(
            source.is_char_boundary(error.span.start) && source.is_char_boundary(error.span.end)
        );
        assert!(!error.message.is_empty());
    }
    let error = CompiledExpression::new("name = \"é\" trailing", false)
        .err()
        .unwrap();
    assert_eq!(&"name = \"é\" trailing"[error.span], "trailing");
}

#[test]
fn source_recursion_token_node_and_pattern_limits_are_bounded() {
    for source in [
        " ".repeat(4097),
        format!("{}kind = file{}", "(".repeat(33), ")".repeat(33)),
        std::iter::repeat_n("kind = file", 300)
            .collect::<Vec<_>>()
            .join(" or "),
        std::iter::repeat_n("name = \"x\"", 17)
            .collect::<Vec<_>>()
            .join(" or "),
        "not ".repeat(40) + "kind = file",
    ] {
        assert!(CompiledExpression::new(&source, false).is_err());
    }
    assert!(matches("name like \"sample.?S\"", &file()));
    assert!(!matches("name like \"sample;*.RS\"", &file()));
}

#[test]
fn diagnostic_categories_distinguish_syntax_type_literal_pattern_and_limit() {
    for (source, expected) in [
        ("name = \"é\" trailing".to_owned(), DiagnosticKind::Syntax),
        ("kind > file".to_owned(), DiagnosticKind::Type),
        (
            "size > 18446744073709551616".to_owned(),
            DiagnosticKind::Literal,
        ),
        ("name =~ \"[\"".to_owned(), DiagnosticKind::Pattern),
        (" ".repeat(4097), DiagnosticKind::Limit),
    ] {
        assert_eq!(
            CompiledExpression::new(&source, false).err().unwrap().kind,
            expected
        );
    }
}

#[test]
fn three_valued_truth_tables_preserve_unknown_under_boolean_composition() {
    let t = "kind = file";
    let f = "kind = directory";
    let u = "attributes has hidden";
    for (left, right, and, or) in [
        (t, t, Some(true), Some(true)),
        (t, f, Some(false), Some(true)),
        (t, u, None, Some(true)),
        (f, t, Some(false), Some(true)),
        (f, f, Some(false), Some(false)),
        (f, u, Some(false), None),
        (u, t, None, Some(true)),
        (u, f, Some(false), None),
        (u, u, None, None),
    ] {
        for (operator, expected) in [("and", and), ("or", or)] {
            let expression = format!("({left}) {operator} ({right})");
            assert_eq!(
                matches(&expression, &file()),
                expected == Some(true),
                "{expression}"
            );
            assert_eq!(
                matches(&format!("not ({expression})"), &file()),
                expected == Some(false),
                "not {expression}"
            );
        }
    }
}

fn assert_unrepresentable_name(name: std::ffi::OsString) {
    let mut entry = file();
    entry.name = name;
    assert!(matches(
        "missing(name) and kind = file and size >= 10MiB",
        &entry
    ));
    assert!(!matches("name != \"safe\"", &entry));
    assert!(!matches("not name like \"*\"", &entry));
    assert!(matches("missing(name) or name =~ \".*\"", &entry));
}

#[test]
fn exact_literal_dates_do_not_round_to_the_native_clock_resolution() {
    let entry = file();
    assert!(matches(
        "modified > \"1969-12-31T23:59:59.999999999Z\"",
        &entry
    ));
    assert!(matches(
        "modified < \"1970-01-01T00:00:00.000000001Z\"",
        &entry
    ));
    assert!(matches("modified = \"1969-12-31T23:30:00-00:30\"", &entry));
    assert!(!matches("modified = \"0001-01-01T00:00:00Z\"", &entry));
    assert!(!matches(
        "modified = \"9999-12-31T23:59:59.999999999Z\"",
        &entry
    ));
}

#[test]
fn pattern_budget_and_strings_cannot_escape_into_executable_behavior() {
    assert_eq!(
        CompiledExpression::new("name =~ \"(?:a{1000}){1000}\"", false)
            .err()
            .unwrap()
            .kind,
        DiagnosticKind::Pattern
    );
    let mut entry = file();
    entry.name = "$(touch owned);${active.path}&é\n.rs".into();
    assert!(matches(
        "name like \"$(touch owned);${active.path}&é\\n.rs\"",
        &entry
    ));
    assert!(!matches("name like \"sample;*.RS\"", &entry));
}
