use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn native_failure_distinguishes_unpaired_utf16_paths_with_identical_lossy_captions() {
    let root = std::env::current_dir().unwrap();
    let first = root.join(OsString::from_wide(&[b'x' as u16, 0xd800]));
    let second = root.join(OsString::from_wide(&[b'x' as u16, 0xd801]));
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    assert_ne!(capture_failure(&first), capture_failure(&second));
}
