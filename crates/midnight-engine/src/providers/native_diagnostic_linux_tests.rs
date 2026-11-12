use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn native_failure_distinguishes_invalid_utf8_paths_with_identical_lossy_captions() {
    let root = std::env::current_dir().unwrap();
    let first = root.join(OsString::from_vec(vec![b'x', 0xfe]));
    let second = root.join(OsString::from_vec(vec![b'x', 0xff]));
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    assert_ne!(capture_failure(&first), capture_failure(&second));
}
