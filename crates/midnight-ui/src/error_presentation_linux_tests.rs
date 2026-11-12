use super::describe;
use midnight_core::FsError;
use std::{ffi::OsString, io, os::unix::ffi::OsStringExt, path::PathBuf};

#[test]
fn distinct_invalid_utf8_paths_keep_distinct_readable_error_details() {
    let path = |byte| {
        let mut bytes = "資料/file-".as_bytes().to_vec();
        bytes.push(byte);
        PathBuf::from(OsString::from_vec(bytes))
    };
    let first = path(0xff);
    let second = path(0xfe);
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    let first_error = FsError::from_io("open", &first, io::Error::other("Access denied"));
    let second_error = FsError::from_io("open", &second, io::Error::other("Access denied"));
    let first_details = describe(&first_error);
    let second_details = describe(&second_error);
    assert_ne!(
        first_details, second_details,
        "Different raw paths must not collapse into one error"
    );
    assert!(!first_details.contains('\u{fffd}'));
    assert!(!second_details.contains('\u{fffd}'));
    assert!(first_details.contains("資料/file-"));
    assert_eq!(first_error.path, first);
    assert_eq!(second_error.path, second);
}
