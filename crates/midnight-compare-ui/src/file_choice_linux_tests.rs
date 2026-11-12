use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn untouched_path_keeps_non_utf8_bytes_and_edits_replace_it_explicitly() {
    let raw = PathBuf::from(OsString::from_vec(b"/tmp/\xff.txt".to_vec()));
    assert_eq!(edited_path(&raw, &raw.to_string_lossy()), raw);
    assert_eq!(
        edited_path(&raw, "/tmp/世界.txt"),
        PathBuf::from("/tmp/世界.txt")
    );
    assert!(edited_path(&raw, "").as_os_str().is_empty());
}
