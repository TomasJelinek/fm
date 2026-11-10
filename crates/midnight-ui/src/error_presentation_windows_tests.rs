use super::describe;
use midnight_core::FsError;
use std::{ffi::OsString, io, os::windows::ffi::OsStringExt, path::PathBuf};

#[test]
fn distinct_unpaired_utf16_paths_keep_distinct_readable_error_details() {
    let path = |unit| {
        let mut units: Vec<_> = "資料/file-".encode_utf16().collect();
        units.push(unit);
        PathBuf::from(OsString::from_wide(&units))
    };
    let first = path(0xd800);
    let second = path(0xd801);
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
