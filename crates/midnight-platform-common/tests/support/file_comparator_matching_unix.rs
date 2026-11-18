use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn either_original_non_utf8_basename_uses_automatic() {
    let policy = compile(vec![rule("*", FileComparatorTarget::BuiltInText)]);
    let raw = PathBuf::from(OsString::from_vec(b"raw\xff.txt".to_vec()));
    let valid = PathBuf::from("valid.txt");
    assert!(raw.file_name().unwrap().to_str().is_none());
    assert_eq!(policy.select(&[raw.clone(), valid.clone()]), None);
    assert_eq!(policy.select(&[valid.clone(), raw.clone()]), None);
    let unicode_basename = raw.join("valid.txt");
    assert_eq!(policy.select(&[unicode_basename, valid]), Some(0));
}
