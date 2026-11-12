use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
pub(super) fn bytes(value: &OsStr) -> Vec<u8> {
    value.encode_wide().flat_map(u16::to_le_bytes).collect()
}
