use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
};
pub(super) fn units(name: &OsStr) -> Vec<u8> {
    name.as_bytes().to_vec()
}
pub(super) fn decode(units: &[u8]) -> Option<String> {
    String::from_utf8(units.to_vec()).ok()
}
pub(super) fn name(units: &[u8]) -> OsString {
    OsString::from_vec(units.to_vec())
}
