#![cfg(unix)]
use midnight_core::{CaseChange, CaseMode, CaseScope};
use midnight_platform_common::metadata::case_name;
use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
#[test]
fn invalid_unselected_bytes_are_preserved_exactly() {
    let name = OsStr::from_bytes(b"\xff.TXT");
    let result = case_name(
        name,
        CaseChange {
            mode: CaseMode::Lower,
            scope: CaseScope::Extension,
        },
    )
    .unwrap();
    assert_eq!(result.as_bytes(), b"\xff.txt");
    assert!(case_name(name, CaseChange::default()).is_err());
}
