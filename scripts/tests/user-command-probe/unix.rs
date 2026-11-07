use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
pub(super) fn bytes(value: &OsStr) -> Vec<u8> {
    value.as_bytes().to_owned()
}
