use std::{ffi::OsString, os::unix::ffi::OsStringExt};

pub fn invalid_name() -> OsString {
    OsString::from_vec(vec![b'a', 0xff])
}
