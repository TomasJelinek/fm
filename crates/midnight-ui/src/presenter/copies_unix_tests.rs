use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn conflict_keeps_unchanged_non_utf8_basename() {
    super::tests::assert_unchanged_display_preserves_native_name(OsString::from_vec(
        b"file-\xff.txt".to_vec(),
    ));
}
