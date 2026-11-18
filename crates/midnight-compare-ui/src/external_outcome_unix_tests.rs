//! Raw Unix path fixtures; no filesystem objects are created.
use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

pub(super) fn paths() -> [PathBuf; 2] {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("_build/verification/viewer-refresh-20261117/ui/external-prep/raw-drafts");
    [
        root.join(OsString::from_vec(b"a-\xff.dat".to_vec())),
        root.join(OsString::from_vec(b"b-\xfe.dat".to_vec())),
    ]
}
