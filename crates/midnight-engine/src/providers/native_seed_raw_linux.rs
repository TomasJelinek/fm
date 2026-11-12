use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn native_seed_keeps_non_utf8_raw_basename() {
    super::assert_raw_seed_name(OsString::from_vec(vec![0xff, b'x', 0x80]));
}
