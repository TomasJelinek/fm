use midnight_core::{CaseChange, CaseMode, CaseScope};
use midnight_platform_common::metadata::case_name;
use std::ffi::OsStr;

fn change(name: &str, mode: CaseMode, scope: CaseScope) -> String {
    case_name(OsStr::new(name), CaseChange { mode, scope })
        .unwrap()
        .into_string()
        .unwrap()
}

#[test]
fn unicode_expansion_and_context_are_preserved() {
    assert_eq!(
        change("straße.ΟΔΟΣ", CaseMode::Upper, CaseScope::Whole),
        "STRASSE.ΟΔΟΣ"
    );
    // Dot is case-ignorable; following TXT keeps sigma medial for whole-name casing.
    assert_eq!(
        change("ΟΔΟΣ.TXT", CaseMode::Lower, CaseScope::Whole),
        "οδοσ.txt"
    );
    assert_eq!(
        change("ΟΔΟΣ.TXT", CaseMode::Lower, CaseScope::Basename),
        "οδος.TXT"
    );
    assert_eq!(change("ΟΔΟΣ", CaseMode::Lower, CaseScope::Whole), "οδος");
    assert_eq!(
        change("İ.TXT", CaseMode::Lower, CaseScope::Whole),
        "i\u{307}.txt"
    );
}

#[test]
fn mixed_modes_and_scopes_match_the_documented_boundaries() {
    assert_eq!(
        change("ßAMPLE DOCUMENT.TAR.GZ", CaseMode::Mixed, CaseScope::Whole),
        "Ssample Document.Tar.Gz"
    );
    assert_eq!(
        change(
            "MY DOCUMENT.TAR.GZ",
            CaseMode::MixedNameLowerExtension,
            CaseScope::Whole
        ),
        "My Document.tar.gz"
    );
    assert_eq!(
        change(
            "MY DOCUMENT.TAR.GZ",
            CaseMode::MixedNameLowerExtension,
            CaseScope::Basename
        ),
        "My Document.Tar.GZ"
    );
    assert_eq!(
        change(
            "MY DOCUMENT.TAR.GZ",
            CaseMode::MixedNameLowerExtension,
            CaseScope::Extension
        ),
        "MY DOCUMENT.TAR.gz"
    );
}

#[test]
fn dotfiles_and_empty_extensions_are_not_reinterpreted() {
    assert_eq!(
        change(".PROFILE", CaseMode::Lower, CaseScope::Basename),
        ".profile"
    );
    assert_eq!(
        change(".PROFILE", CaseMode::Lower, CaseScope::Extension),
        ".PROFILE"
    );
    assert_eq!(
        change("NAME.", CaseMode::Lower, CaseScope::Extension),
        "NAME."
    );
    assert_eq!(
        change("NAME.TXT", CaseMode::Lower, CaseScope::Basename),
        "name.TXT"
    );
}
