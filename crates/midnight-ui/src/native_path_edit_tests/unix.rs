use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

pub(super) fn raw_directory() -> PathBuf {
    PathBuf::from(OsString::from_vec(b"/qa/raw-\xff".to_vec()))
}
pub(super) fn plain_directory() -> PathBuf {
    PathBuf::from("/qa/plain")
}
