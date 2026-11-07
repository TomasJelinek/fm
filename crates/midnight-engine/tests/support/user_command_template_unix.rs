use super::*;
use std::os::unix::ffi::{OsStrExt, OsStringExt};

#[test]
fn invalid_utf8_native_names_round_trip_inside_literal_arguments() {
    let path = OsString::from_vec(b"file-\xff\xfe-${other.path}".to_vec());
    let result = Template::parse("before:${file.path}:after")
        .unwrap()
        .expand(|variable| (variable == Variable::FilePath).then_some(path.as_os_str()))
        .unwrap();
    assert_eq!(
        result.as_bytes(),
        b"before:file-\xff\xfe-${other.path}:after"
    );
}
