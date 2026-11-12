use std::{ffi::OsString, os::windows::ffi::OsStringExt};

pub fn invalid_name() -> OsString {
    OsString::from_wide(&[b'a' as u16, 0xd800])
}
