//! Distinct native targets that deliberately share one lossy UI spelling.
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

pub(super) fn same_display(root: &Path) -> [PathBuf; 2] {
    [0xd800, 0xd801].map(|unit| {
        let mut path: Vec<u16> = root.as_os_str().encode_wide().collect();
        path.extend([b'\\' as u16, unit]);
        PathBuf::from(OsString::from_wide(&path))
    })
}
