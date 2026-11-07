use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn copy_dialog_keeps_unchanged_non_utf8_destination() {
    super::super::tests::copy_dialog::assert_native_destination_is_preserved(OsString::from_vec(
        b"dest-\xff".to_vec(),
    ));
}

#[test]
fn conflict_keeps_unchanged_non_utf8_basename() {
    super::tests::assert_unchanged_display_preserves_native_name(OsString::from_vec(
        b"file-\xff.txt".to_vec(),
    ));
}
