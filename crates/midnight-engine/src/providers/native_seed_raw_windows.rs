use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn native_seed_keeps_unpaired_utf16_and_long_basename() {
    let mut units = vec![0xd800];
    units.extend(std::iter::repeat_n(b'x' as u16, 300));
    super::assert_raw_seed_name(OsString::from_wide(&units));
}
