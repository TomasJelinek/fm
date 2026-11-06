use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn conflict_keeps_unchanged_unpaired_utf16_basename() {
    super::tests::assert_unchanged_display_preserves_native_name(OsString::from_wide(&[
        102, 105, 108, 101, 45, 0xd800, 46, 116, 120, 116,
    ]));
}
