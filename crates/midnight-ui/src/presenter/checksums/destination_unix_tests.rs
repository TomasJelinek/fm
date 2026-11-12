use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

pub(super) fn raw_parent() -> PathBuf {
    PathBuf::from(OsString::from_vec(b"/qa/raw-\xff".to_vec()))
}

pub(super) fn plain_parent() -> PathBuf {
    PathBuf::from("/qa/plain")
}
