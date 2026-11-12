use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn raw_unix_name_is_unknown_without_losing_metadata_predicates() {
    super::assert_unrepresentable_name(OsString::from_vec(vec![0xFF, b'.', b'r', b's']));
}
