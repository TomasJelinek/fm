//! Distinct native targets that deliberately share one lossy UI spelling.
use std::{
    ffi::OsString,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

pub(super) fn same_display(root: &Path) -> [PathBuf; 2] {
    [0x80, 0x81].map(|byte| {
        let mut path = root.as_os_str().as_bytes().to_vec();
        path.extend([b'/', byte]);
        PathBuf::from(OsString::from_vec(path))
    })
}
