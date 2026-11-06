use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
};
pub(super) fn units(name: &OsStr) -> Vec<u16> {
    name.encode_wide().collect()
}
pub(super) fn decode(units: &[u16]) -> Option<String> {
    String::from_utf16(units).ok()
}
pub(super) fn name(units: &[u16]) -> OsString {
    OsString::from_wide(units)
}
