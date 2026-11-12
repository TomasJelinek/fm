use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn raw_utf16_name_is_unknown_without_losing_metadata_predicates() {
    super::assert_unrepresentable_name(OsString::from_wide(&[0xD800, 0x2e, 0x72, 0x73]));
}
