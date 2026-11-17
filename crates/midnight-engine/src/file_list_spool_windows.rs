//! `%TEMP%` is per-user on Windows (the profile's `Local\Temp`). The private-root
//! check still refuses a directory that another account owns or can access.
use std::path::PathBuf;

pub(super) fn candidates() -> Vec<PathBuf> {
    vec![std::env::temp_dir().join("MidnightExplorer-file-list")]
}
