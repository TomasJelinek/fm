use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn either_original_unpaired_surrogate_basename_uses_automatic() {
    let policy = compile(vec![rule("*", FileComparatorTarget::BuiltInText)]);
    let raw = PathBuf::from(OsString::from_wide(&[0xd800, 0x2e, 0x74, 0x78, 0x74]));
    let valid = PathBuf::from("valid.txt");
    assert!(raw.file_name().unwrap().to_str().is_none());
    assert_eq!(policy.select(&[raw.clone(), valid.clone()]), None);
    assert_eq!(policy.select(&[valid.clone(), raw.clone()]), None);
    // A permissive panel-filter fallback must never authorize a custom comparator.
    let unicode_basename = raw.join("valid.txt");
    assert_eq!(policy.select(&[unicode_basename, valid]), Some(0));
}
