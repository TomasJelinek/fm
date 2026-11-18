use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

#[test]
fn non_utf8_program_and_cwd_roundtrip_losslessly() {
    let path = PathBuf::from(OsString::from_vec(b"/tools/compare-\xff".to_vec()));
    let mut expected = preferences();
    let FileComparatorTarget::External(program) = &mut expected.file_comparators.rules[3].target
    else {
        panic!()
    };
    program.executable = path.clone();
    program.working_directory = Some(path);
    let bytes = encode_preferences(expected.clone()).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("unix-bytes"));
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);
}
