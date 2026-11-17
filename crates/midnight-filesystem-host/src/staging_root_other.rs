//! The platform temporary directory is already per-user here (Windows `%TEMP%`,
//! macOS `$TMPDIR`); the private-root check still verifies ownership and access.
use std::path::PathBuf;

pub(super) fn candidates() -> Vec<PathBuf> {
    vec![std::env::temp_dir().join("MidnightExplorer-archive-spool")]
}
