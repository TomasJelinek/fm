#![cfg(windows)]
use midnight_core::{CaseChange, CaseMode, CaseScope};
use midnight_platform_common::metadata::case_name;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
#[test]
fn invalid_unselected_utf16_is_preserved_exactly() {
    let name = std::ffi::OsString::from_wide(&[0xd800, 46, 84, 88, 84]);
    let result = case_name(
        &name,
        CaseChange {
            mode: CaseMode::Lower,
            scope: CaseScope::Extension,
        },
    )
    .unwrap();
    assert_eq!(
        result.encode_wide().collect::<Vec<_>>(),
        [0xd800, 46, 116, 120, 116]
    );
    assert!(case_name(&name, CaseChange::default()).is_err());
}
