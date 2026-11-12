use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn raw_native_names_route_by_ascii_archive_suffix() {
    for (suffix, expected) in [
        (b".ZIP".as_slice(), Format::Zip),
        (b".TaR.Gz", Format::TarGzip),
        (b".tar", Format::Tar),
    ] {
        let name = OsString::from_vec([b"raw-\xff".as_slice(), suffix].concat());
        assert_eq!(
            catalog().choose(Path::new(&name)).map(|(_, format)| format),
            Some(expected)
        );
    }
}
