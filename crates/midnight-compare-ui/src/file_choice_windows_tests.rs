use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn untouched_path_keeps_unpaired_utf16_and_edits_replace_it_explicitly() {
    let raw = PathBuf::from(OsString::from_wide(&[b'C' as u16, 58, 92, 0xd800, 46, 116]));
    assert_eq!(edited_path(&raw, &raw.to_string_lossy()), raw);
    assert_eq!(
        edited_path(&raw, "D:\\世界.txt"),
        PathBuf::from("D:\\世界.txt")
    );
    assert!(edited_path(&raw, "").as_os_str().is_empty());
}
