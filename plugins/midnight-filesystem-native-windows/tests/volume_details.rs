#![cfg(windows)]
mod support;
use midnight_filesystem_plugin_api::{abi::*, *};
use support::{Client, Temp, address};

#[test]
fn native_volume_details_preserve_capacity_label_and_failures() {
    let fixture = Temp::new();
    let client = Client::new();
    assert_ne!(client.api.capabilities & CAP_VOLUME_DETAILS, 0);
    let directory = fixture.path.join("Žluťoučký_文档_🦀");
    std::fs::create_dir(&directory).unwrap();
    let request = |path: &std::path::Path| Request::VolumeInformation {
        address: address(path),
    };
    let Response::VolumeInformation(info) = client.call(request(&directory)) else {
        panic!("Missing rich native information")
    };
    // The root is named as Win32 names it (for example C:\), not \\?\C:\.
    let root = info.root.as_ref().unwrap();
    let units: Vec<u16> = root
        .bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|unit| u16::from_le_bytes(*unit))
        .collect();
    let root = String::from_utf16(&units).unwrap().to_lowercase();
    let fixture_path = win32_form(
        &std::fs::canonicalize(&fixture.path)
            .unwrap()
            .to_string_lossy()
            .to_lowercase(),
    );
    assert!(!root.starts_with(r"\\?\"), "{root}");
    assert!(
        fixture_path.starts_with(&root),
        "{root} must contain {fixture_path}"
    );
    assert!(
        info.filesystem
            .as_ref()
            .is_some_and(|text| !text.bytes.is_empty())
    );
    assert!(info.capacity.total.is_some_and(|total| total > 0));
    assert!(info.capacity.free.unwrap() <= info.capacity.total.unwrap());
    assert!(info.capacity.available.is_some());
    assert!(info.read_only.is_some());
    assert_eq!(
        client.call(Request::VolumeLabel {
            address: address(&directory)
        }),
        Response::VolumeLabel(info.label)
    );
    assert!(matches!(
        client.call(request(&directory.join("missing"))),
        Response::Failed(Failure {
            code: ErrorCode::NotFound,
            ..
        })
    ));
    assert!(matches!(
        client.cancelled(request(&directory), 1),
        Response::Failed(Failure {
            code: ErrorCode::Cancelled,
            ..
        })
    ));
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
}

#[test]
fn native_volume_details_accept_raw_and_long_directory_identities() {
    let fixture = Temp::new();
    let client = Client::new();
    let raw = fixture.path.join(raw_name());
    std::fs::create_dir(&raw).unwrap();
    let mut long = raw.clone();
    for _ in 0..5 {
        long = long.join("long_資料_".repeat(10));
    }
    std::fs::create_dir_all(&long).unwrap();
    for path in [&raw, &long] {
        assert!(matches!(
            client.call(Request::VolumeInformation {
                address: address(path)
            }),
            Response::VolumeInformation(_)
        ));
    }
}
/// The Win32 form the plugin publishes for a lowercased verbatim path:
/// `\\?\unc\server\share\...` becomes `\\server\share\...` (a checkout on a
/// share or mapped drive) and `\\?\c:\...` becomes `c:\...`.
fn win32_form(verbatim: &str) -> String {
    match verbatim.strip_prefix(r"\\?\unc\") {
        Some(share) => format!(r"\\{share}"),
        None => verbatim.trim_start_matches(r"\\?\").to_owned(),
    }
}
#[test]
fn verbatim_fixture_paths_compare_in_their_win32_form() {
    assert_eq!(win32_form(r"\\?\c:\work\x"), r"c:\work\x");
    assert_eq!(win32_form(r"\\?\unc\server\share\x"), r"\\server\share\x");
    assert_eq!(win32_form(r"c:\work\x"), r"c:\work\x");
}
fn raw_name() -> std::ffi::OsString {
    use std::os::windows::ffi::OsStringExt;
    std::ffi::OsString::from_wide(&[b'r' as u16, 0xd800])
}
