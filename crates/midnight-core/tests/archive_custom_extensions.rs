use midnight_core::{
    ARCHIVE_FORMATS, ArchiveExtensionHints, ArchiveFormat, UserPreferences,
    normalize_archive_extension, validate_archive_custom_extensions,
};
use std::path::Path;

#[test]
fn literal_unicode_suffixes_fold_without_rewriting_spelling_or_interpreting_syntax() {
    for (input, expected) in [
        (".BACKUP.TAR", "backup.tar"),
        (" ŽLUŤOUČKÝ ", "žluťoučký"),
        ("Straße", "strasse"),
        ("Σς", "σσ"),
        ("文档.🗃", "文档.🗃"),
        ("c++", "c++"),
        ("e\u{301}", "e\u{301}"),
    ] {
        assert_eq!(normalize_archive_extension(input).unwrap(), expected);
    }
    assert_ne!(
        normalize_archive_extension("é").unwrap(),
        normalize_archive_extension("e\u{301}").unwrap()
    );
    for input in [
        "",
        ".",
        "..zip",
        "foo..zip",
        "foo.",
        "*.zip",
        "a?b",
        "[ab]",
        "a/b",
        "a\\b",
        "c:zip",
        "a\0b",
        "$(whoami)",
        "a;b",
        "a b",
    ] {
        assert!(normalize_archive_extension(input).is_err(), "{input:?}");
    }
    assert!(normalize_archive_extension(&"x".repeat(257)).is_err());
}

#[test]
fn longest_literal_suffix_wins_and_disabled_longer_builtin_blocks_shorter_assignment() {
    let mut preferences = UserPreferences::default();
    preferences.archive_custom_extensions.extend([
        ("backup".into(), ArchiveFormat::Zip),
        ("tar.backup".into(), ArchiveFormat::TarGzip),
        ("special.gz".into(), ArchiveFormat::SevenZip),
        ("strasse".into(), ArchiveFormat::Xz),
    ]);
    for (name, expected) in [
        ("文档.BACKUP", Some(ArchiveFormat::Zip)),
        ("source.TAR.BACKUP", Some(ArchiveFormat::TarGzip)),
        ("source.SPECIAL.GZ", Some(ArchiveFormat::SevenZip)),
        ("source.tar.gz", Some(ArchiveFormat::TarGzip)),
        ("file.STRAẞE", Some(ArchiveFormat::Xz)),
        ("file.backup.exe", None),
        (".backup", None),
    ] {
        assert_eq!(
            ArchiveExtensionHints::from_path(Path::new(name)).enabled_format(&preferences),
            expected,
            "{name}"
        );
    }
    preferences
        .archive_association_overrides
        .insert("tar.gz".into(), false);
    assert_eq!(
        ArchiveExtensionHints::from_path(Path::new("source.tar.gz")).enabled_format(&preferences),
        None
    );
    let long = format!("{}/文档.tar.backup", "long/".repeat(2000));
    assert_eq!(
        ArchiveExtensionHints::from_path(Path::new(&long)).enabled_format(&preferences),
        Some(ArchiveFormat::TarGzip)
    );
    preferences
        .archive_custom_extensions
        .insert("k".repeat(100), ArchiveFormat::Zip);
    let wider_case_spelling = format!("file.{}", "K".repeat(100));
    assert_eq!(
        ArchiveExtensionHints::from_path(Path::new(&wider_case_spelling))
            .enabled_format(&preferences),
        Some(ArchiveFormat::Zip)
    );
}

#[test]
fn format_identifiers_round_trip_and_reserved_or_noncanonical_settings_fail() {
    let mut preferences = UserPreferences::default();
    for (index, format) in ARCHIVE_FORMATS.iter().copied().enumerate() {
        assert_eq!(ArchiveFormat::from_id(format.id()), Some(format));
        preferences
            .archive_custom_extensions
            .insert(format!("format{index}"), format);
    }
    validate_archive_custom_extensions(&preferences.archive_custom_extensions).unwrap();
    for extension in [
        "docx", "zip", "tar.gz", ".custom", "UPPER", "Straße", "bad..ext",
    ] {
        let mut extensions = preferences.archive_custom_extensions.clone();
        extensions.insert(extension.into(), ArchiveFormat::Tar);
        assert!(
            validate_archive_custom_extensions(&extensions).is_err(),
            "{extension}"
        );
    }
    for index in 0..257 {
        preferences
            .archive_custom_extensions
            .insert(format!("x{index}"), ArchiveFormat::Zip);
    }
    assert!(validate_archive_custom_extensions(&preferences.archive_custom_extensions).is_err());
}
