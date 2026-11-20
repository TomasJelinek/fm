use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

pub(in crate::presenter) fn raw_parent() -> PathBuf {
    PathBuf::from(OsString::from_vec(b"/qa/raw-\xff".to_vec()))
}

pub(in crate::presenter) fn plain_parent() -> PathBuf {
    PathBuf::from("/qa/plain")
}
