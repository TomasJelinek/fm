//! Raw Windows path fixtures; no filesystem objects are created.
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};

pub(super) fn paths() -> [PathBuf; 2] {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("_build/verification/viewer-refresh-20261117/ui/external-prep/raw-drafts");
    [
        [b'a' as u16, 0xd800, 46, 100, 97, 116],
        [b'b' as u16, 0xdc00, 46, 100, 97, 116],
    ]
    .map(|units| root.join(OsString::from_wide(&units)))
}
