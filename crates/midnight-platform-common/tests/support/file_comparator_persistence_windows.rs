use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};

#[test]
fn malformed_utf16_program_and_cwd_roundtrip_losslessly() {
    let path = PathBuf::from(OsString::from_wide(&[
        67, 58, 92, 0xD800, 92, 116, 46, 101, 120, 101,
    ]));
    let mut expected = preferences();
    let FileComparatorTarget::External(program) = &mut expected.file_comparators.rules[3].target
    else {
        panic!()
    };
    program.executable = path.clone();
    program.working_directory = Some(path);
    let bytes = encode_preferences(expected.clone()).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("windows-utf16"));
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);
}
