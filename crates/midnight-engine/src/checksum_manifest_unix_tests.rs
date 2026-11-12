use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn raw_unix_names_are_refused_and_literal_backslashes_are_gnu_escaped() {
    let (mut run, request) = fixture("original");
    let row = Arc::make_mut(&mut Arc::make_mut(&mut run.rows)[0]);
    row.path.set_file_name(OsString::from_vec(vec![b'x', 0xff]));
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
    let row = Arc::make_mut(&mut Arc::make_mut(&mut run.rows)[0]);
    row.path.set_file_name("literal\\name");
    let (bytes, _) = encode(
        &run,
        &request,
        request.destination.parent().unwrap(),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        format!("\\{} *literal\\\\name\n", "0".repeat(64))
    );
}

#[test]
fn escaped_literal_backslashes_round_trip_without_becoming_directories() {
    let (run, request) = fixture("literal\\name");
    let (bytes, _) = encode(
        &run,
        &request,
        request.destination.parent().unwrap(),
        &Cancellation::default(),
    )
    .unwrap();
    let parsed = super::super::super::format::parse_manifest(
        &bytes,
        &request.destination,
        ChecksumLimits::default(),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(parsed.entries[0].path, run.rows[0].path);
    assert_ne!(
        parsed.entries[0].path,
        request.destination.parent().unwrap().join("literal/name")
    );
}
