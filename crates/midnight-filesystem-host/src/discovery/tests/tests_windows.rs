use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn raw_native_names_route_by_ascii_archive_suffix() {
    for (suffix, expected) in [
        (".ZIP", Format::Zip),
        (".TaR.Gz", Format::TarGzip),
        (".tar", Format::Tar),
    ] {
        let name = OsString::from_wide(
            &[vec![b'r' as u16, 0xd800], suffix.encode_utf16().collect()].concat(),
        );
        assert_eq!(
            catalog().choose(Path::new(&name)).map(|(_, format)| format),
            Some(expected)
        );
    }
}
