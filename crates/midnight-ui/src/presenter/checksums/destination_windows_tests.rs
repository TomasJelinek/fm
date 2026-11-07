use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};

pub(super) fn raw_parent() -> PathBuf {
    let mut units: Vec<u16> = r"C:\qa\raw-".encode_utf16().collect();
    units.push(0xd800);
    PathBuf::from(OsString::from_wide(&units))
}

pub(super) fn plain_parent() -> PathBuf {
    PathBuf::from(r"C:\qa\plain")
}
