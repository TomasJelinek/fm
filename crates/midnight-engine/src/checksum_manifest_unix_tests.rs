use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn raw_unix_names_are_refused_and_literal_backslashes_are_gnu_escaped() {
    let (mut run, request) = fixture("original");
    let mut row = run.rows[0].clone();
    row.name = PathBuf::from(OsString::from_vec(vec![b'x', 0xff])).into();
    set_rows(&mut run, vec![row.clone()]);
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
    row.name = Path::new("literal\\name").into();
    set_rows(&mut run, vec![row]);
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
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(parsed.names[0], run.rows[0].name);
    assert_ne!(&*parsed.names[0], Path::new("literal/name"));
}
