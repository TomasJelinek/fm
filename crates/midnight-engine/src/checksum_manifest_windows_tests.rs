use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn unpaired_native_utf16_is_refused_without_replacement_characters() {
    let (mut run, request) = fixture("original");
    let row = Arc::make_mut(&mut Arc::make_mut(&mut run.rows)[0]);
    row.path
        .set_file_name(OsString::from_wide(&[b'x' as u16, 0xd800]));
    assert_eq!(
        encode(
            &run,
            &request,
            request.destination.parent().unwrap(),
            &Cancellation::default()
        )
        .unwrap_err()
        .kind,
        io::ErrorKind::InvalidData
    );
}
