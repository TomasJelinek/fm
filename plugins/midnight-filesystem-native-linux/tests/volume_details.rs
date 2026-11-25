#![cfg(target_os = "linux")]
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
    assert!(info.root.is_some());
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
/// The drive list through the provider's request: every drive named for the drive
/// toolbar, never by its mount path, with its whole root kept as its address. Without
/// extra mounts (a plain container) only `/` is checked; tmpfs mounts such as
/// `/mnt/USB` and `/media/alice/USB` show `mnt/USB` and `alice/USB`.
#[test]
fn listed_volumes_show_names_of_their_own_and_keep_whole_roots() {
    let client = Client::new();
    let Response::Volumes(volumes) = client.call(Request::Volumes) else {
        panic!("Missing native volume list")
    };
    let root = volumes
        .iter()
        .find(|volume| volume.root.bytes == b"/")
        .expect("the root is always listed");
    assert_eq!((root.id.as_str(), root.display.as_str()), ("mount:2f", "/"));
    let mut names = std::collections::HashSet::new();
    for volume in &volumes {
        let path = String::from_utf8_lossy(&volume.root.bytes);
        assert!(
            names.insert(volume.display.as_str()),
            "two drives show {:?}",
            volume.display
        );
        assert!(
            volume.root.bytes == b"/" || !volume.display.starts_with('/'),
            "{path} shows {:?}, not a name of its own",
            volume.display
        );
        assert_eq!(
            volume.id,
            format!(
                "mount:{}",
                volume
                    .root
                    .bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
            "{path}"
        );
    }
    for (path, name) in [("/mnt/USB", "mnt/USB"), ("/media/alice/USB", "alice/USB")] {
        if let Some(volume) = volumes
            .iter()
            .find(|volume| volume.root.bytes == path.as_bytes())
        {
            assert_eq!(volume.display, name);
        }
    }
}

fn raw_name() -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(b"raw_\xff".to_vec())
}
